/*
Pure translator: Claude Agent SDK JSONL records -> CRP JSONL events.

Contract:
- translateClaudeEventsToCrp(records, opts) -> CrpEvent[] (no IO, deterministic)
- Input records mirror capture.mjs output:
  - { record: "header", ... }
  - { record: "event", event: { type: "system"|"stream_event"|"assistant"|"user"|"result", ... } }
  - { record: "end", interrupted?: boolean, ... }

Supported input event shapes (fixtures):
- system:init
- system:status
- system:compact_boundary
- system:local_command_output
- stream_event: message_start, content_block_start, content_block_delta, content_block_stop,
  message_delta, message_stop
- assistant (final message)
- user (tool_result)
- result (run completion)

CRP output events (control plane unless noted):
- turn.started
- message.delta (data plane)
- message.final
- tool.started
- tool.completed
- turn.completed
- session.notice (model.mismatch)
- session.notice (context.compacted / context.compacting)

Ordering invariants:
- turn.started emitted once per translation before any message/tool events.
- tool.started emitted when tool input JSON parses (or on block stop fallback).
- tool.completed emitted after corresponding tool.started (forced if missing).
- message.final emitted for assistant text-only content (skip pure tool_use messages).
- turn.completed emitted on result (status derived from result/interruption markers).

Note: translate.ts is intentionally JS-compatible (no TS-only syntax) so the Node
scripts can load it without a build step.
*/

const DEFAULT_MAX_TOOL_INPUT_BYTES = 64 * 1024;

export function translateClaudeEventsToCrp(records, opts = {}) {
  const events = [];
  let seq = 0;

  let sessionId = opts.sessionId || null;
  let requestedModel = opts.requestedModel || null;
  let actualModel = null;
  let runId = opts.runId || null;
  let turnId = opts.turnId || null;
  let turnStarted = false;
  let modelNoticeEmitted = false;
  let interrupted = false;
  let endSeen = false;

  const maxToolInputBytes =
    typeof opts.maxToolInputBytes === "number" && opts.maxToolInputBytes > 0
      ? opts.maxToolInputBytes
      : DEFAULT_MAX_TOOL_INPUT_BYTES;

  const toolStates = new Map(); // tool_call_id -> state
  const blockIndexToToolId = new Map();
  const finalMessageIds = new Set();
  let activeMessageId = null;

  function emit(channel, payload) {
    seq += 1;
    events.push({ v: 1, seq, channel, ...payload });
  }

  function ensureIds() {
    if (!sessionId) return false;
    if (!turnId) turnId = `turn_${sessionId}`;
    if (!runId) runId = `run_${turnId}`;
    return true;
  }

  function ensureTurnStarted() {
    if (!ensureIds()) return;
    if (!turnStarted) {
      emit("control", {
        type: "turn.started",
        session_id: sessionId,
        run_id: runId,
        turn_id: turnId
      });
      turnStarted = true;
    }
    maybeEmitModelMismatch();
  }

  function maybeEmitModelMismatch() {
    if (modelNoticeEmitted) return;
    if (!requestedModel || !actualModel || !sessionId) return;
    if (requestedModel === actualModel) return;
    emit("control", {
      type: "session.notice",
      session_id: sessionId,
      turn_id: turnStarted ? turnId : undefined,
      code: "model.mismatch",
      severity: "warning",
      message: `Requested model ${requestedModel} but got ${actualModel}.`,
      details: { requested_model: requestedModel, actual_model: actualModel }
    });
    modelNoticeEmitted = true;
  }

  function hasNonEmptyObject(value) {
    return value && typeof value === "object" && Object.keys(value).length > 0;
  }

  function sliceByBytes(text, maxBytes) {
    if (Buffer.byteLength(text, "utf8") <= maxBytes) return text;
    let bytes = 0;
    let idx = 0;
    for (const ch of text) {
      const size = Buffer.byteLength(ch, "utf8");
      if (bytes + size > maxBytes) break;
      bytes += size;
      idx += ch.length;
    }
    return text.slice(0, idx);
  }

  function tryParseJson(text) {
    try {
      return JSON.parse(text);
    } catch (err) {
      return null;
    }
  }

  function emitToolStarted(tool) {
    if (!tool || tool.emitted) return;
    ensureTurnStarted();
    if (!turnId) return;
    const payload = {
      type: "tool.started",
      session_id: sessionId,
      run_id: runId,
      turn_id: turnId,
      tool_call_id: tool.id,
      tool_name: tool.name,
      input: tool.inputParsed || (tool.inputBuffer ? tool.inputBuffer : null)
    };
    if (tool.inputTruncated) payload.input_truncated = true;
    if (tool.inputOriginalBytes != null) {
      payload.input_original_bytes = tool.inputOriginalBytes;
    }
    emit("control", payload);
    tool.emitted = true;
  }

  function appendToolInput(tool, chunk) {
    if (!tool) return;
    const chunkBytes = Buffer.byteLength(chunk, "utf8");
    tool.inputOriginalBytes += chunkBytes;
    if (tool.inputTruncated) return;
    const currentBytes = Buffer.byteLength(tool.inputBuffer, "utf8");
    if (currentBytes + chunkBytes <= maxToolInputBytes) {
      tool.inputBuffer += chunk;
    } else {
      const remaining = Math.max(0, maxToolInputBytes - currentBytes);
      if (remaining > 0) {
        tool.inputBuffer += sliceByBytes(chunk, remaining);
      }
      tool.inputTruncated = true;
    }

    if (!tool.emitted && !tool.inputTruncated) {
      const parsed = tryParseJson(tool.inputBuffer);
      if (parsed !== null) {
        tool.inputParsed = parsed;
        emitToolStarted(tool);
      }
    }
  }

  function handleToolBlockStop(toolId) {
    const tool = toolStates.get(toolId);
    if (!tool || tool.emitted) return;
    if (!tool.inputTruncated && tool.inputParsed == null) {
      const parsed = tryParseJson(tool.inputBuffer);
      if (parsed !== null) {
        tool.inputParsed = parsed;
      }
    }
    emitToolStarted(tool);
  }

  function recordInterruptionMarker(message) {
    if (!message || !Array.isArray(message.content)) return;
    for (const item of message.content) {
      if (item && item.type === "text" && typeof item.text === "string") {
        if (item.text.includes("Request interrupted by user")) {
          interrupted = true;
          ensureTurnStarted();
        }
      }
    }
  }

  for (const record of records || []) {
    if (!record || typeof record !== "object") continue;

    if (record.record === "header") {
      if (!sessionId && record.session_id) sessionId = record.session_id;
      if (!requestedModel && record.model) requestedModel = record.model;
      if (!requestedModel && record.options && record.options.model) {
        requestedModel = record.options.model;
      }
      continue;
    }

    if (record.record === "end") {
      endSeen = true;
      if (record.interrupted) interrupted = true;
      continue;
    }

    if (record.record !== "event") continue;

    const ev = record.event || {};
    if (!sessionId && ev.session_id) sessionId = ev.session_id;

    if (ev.type === "system" && ev.subtype === "init") {
      if (!actualModel && ev.model) actualModel = ev.model;
      maybeEmitModelMismatch();
      continue;
    }

    if (ev.type === "system" && ev.subtype === "status") {
      if (ev.status === "compacting") {
        ensureTurnStarted();
        emit("control", {
          type: "session.notice",
          session_id: sessionId,
          turn_id: turnId,
          code: "context.compacting",
          severity: "info",
          message: "Compacting conversation context.",
          transient: true
        });
      }
      continue;
    }

    if (ev.type === "system" && ev.subtype === "compact_boundary") {
      ensureTurnStarted();
      emit("control", {
        type: "session.notice",
        session_id: sessionId,
        turn_id: turnId,
        code: "context.compacted",
        severity: "info",
        message: "Context compacted. Earlier turns were summarized.",
        details: ev.compact_metadata ? { compact_metadata: ev.compact_metadata } : undefined
      });
      continue;
    }

    if (ev.type === "system" && ev.subtype === "local_command_output") {
      const content = typeof ev.content === "string" ? ev.content : "";
      if (!content) continue;
      ensureTurnStarted();
      const messageId =
        typeof ev.uuid === "string" && ev.uuid ? ev.uuid : `local_command_${seq}`;
      if (finalMessageIds.has(messageId)) continue;
      emit("control", {
        type: "message.final",
        session_id: sessionId,
        run_id: runId,
        turn_id: turnId,
        message_id: messageId,
        content
      });
      finalMessageIds.add(messageId);
      continue;
    }

    if (ev.type === "stream_event" && ev.event) {
      const se = ev.event;
      if (se.type === "message_start" && se.message) {
        activeMessageId = se.message.id || activeMessageId;
        if (!actualModel && se.message.model) actualModel = se.message.model;
        ensureTurnStarted();
        continue;
      }

      if (se.type === "content_block_start") {
        const block = se.content_block || {};
        if (block.type === "tool_use") {
          const toolId = block.id;
          if (toolId) {
            blockIndexToToolId.set(se.index, toolId);
            const state = {
              id: toolId,
              name: block.name || "unknown",
              inputBuffer: "",
              inputParsed: hasNonEmptyObject(block.input) ? block.input : null,
              inputTruncated: false,
              inputOriginalBytes: 0,
              emitted: false
            };
            toolStates.set(toolId, state);
            if (state.inputParsed) {
              emitToolStarted(state);
            }
          }
        }
        continue;
      }

      if (se.type === "content_block_delta" && se.delta) {
        if (se.delta.type === "text_delta") {
          if (activeMessageId) {
            ensureTurnStarted();
            emit("data", {
              type: "message.delta",
              session_id: sessionId,
              run_id: runId,
              turn_id: turnId,
              message_id: activeMessageId,
              delta: se.delta.text || ""
            });
          }
          continue;
        }
        if (se.delta.type === "input_json_delta") {
          const toolId = blockIndexToToolId.get(se.index);
          const tool = toolStates.get(toolId);
          ensureTurnStarted();
          appendToolInput(tool, se.delta.partial_json || "");
          continue;
        }
      }

      if (se.type === "content_block_stop") {
        const toolId = blockIndexToToolId.get(se.index);
        if (toolId) {
          handleToolBlockStop(toolId);
          blockIndexToToolId.delete(se.index);
        }
        continue;
      }

      // message_delta/message_stop currently do not affect CRP output directly.
      continue;
    }

    if (ev.type === "assistant" && ev.message) {
      const message = ev.message;
      if (!actualModel && message.model) actualModel = message.model;
      ensureTurnStarted();
      const messageId = message.id || activeMessageId;
      if (!messageId) continue;
      if (finalMessageIds.has(messageId)) continue;

      let text = "";
      if (Array.isArray(message.content)) {
        for (const item of message.content) {
          if (item && item.type === "text" && typeof item.text === "string") {
            text += item.text;
          }
        }
      }
      if (text) {
        emit("control", {
          type: "message.final",
          session_id: sessionId,
          run_id: runId,
          turn_id: turnId,
          message_id: messageId,
          content: text
        });
        finalMessageIds.add(messageId);
      }
      continue;
    }

    if (ev.type === "user" && ev.message) {
      recordInterruptionMarker(ev.message);
      const content = Array.isArray(ev.message.content) ? ev.message.content : [];
      const toolUseResult = ev.tool_use_result;
      for (const item of content) {
        if (!item || item.type !== "tool_result") continue;
        const toolId = item.tool_use_id;
        if (!toolId) continue;
        const tool = toolStates.get(toolId);
        if (tool && !tool.emitted) emitToolStarted(tool);
        ensureTurnStarted();
        const toolName = tool ? tool.name : "unknown";
        const isError = Boolean(item.is_error);
        let output = null;
        let error = null;
        if (isError) {
          if (typeof toolUseResult === "string") {
            error = toolUseResult;
          } else if (typeof item.content === "string") {
            error = item.content;
          } else {
            error = "tool_error";
          }
        } else {
          if (toolUseResult !== undefined) output = toolUseResult;
          else output = item.content || null;
        }

        emit("control", {
          type: "tool.completed",
          session_id: sessionId,
          run_id: runId,
          turn_id: turnId,
          tool_call_id: toolId,
          tool_name: toolName,
          status: isError ? "error" : "success",
          output,
          error
        });
      }
      continue;
    }

    if (ev.type === "result") {
      ensureTurnStarted();
      const status = interrupted
        ? "canceled"
        : ev.subtype === "success" && ev.is_error === false
          ? "success"
          : "error";
      const payload = {
        type: "turn.completed",
        session_id: sessionId,
        run_id: runId,
        turn_id: turnId,
        status
      };
      if (status === "error") {
        let message = null;
        if (Array.isArray(ev.errors) && ev.errors.length > 0) {
          message = String(ev.errors[0]);
        } else if (typeof ev.error === "string" && ev.error) {
          message = ev.error;
        }
        if (message) {
          payload.error = { message };
        }
      }
      if (ev.usage) payload.usage = ev.usage;
      if (ev.modelUsage) payload.model_usage = ev.modelUsage;
      if (typeof ev.num_turns === "number") payload.num_turns = ev.num_turns;
      if (ev.errors) payload.errors = ev.errors;
      emit("control", payload);
      continue;
    }
  }

  // If the turn started but we never observed a result record, emit an error completion so ctx can
  // finalize the turn (prevents "Working" hangs on early failures).
  const hasTurnCompleted = events.some((e) => e && e.type === "turn.completed");
  if (endSeen && turnStarted && !hasTurnCompleted && ensureIds()) {
    const payload = {
      type: "turn.completed",
      session_id: sessionId,
      run_id: runId,
      turn_id: turnId,
      status: interrupted ? "canceled" : "error"
    };
    if (!interrupted) payload.error = { message: "missing_result" };
    emit("control", payload);
  }

  return events;
}
