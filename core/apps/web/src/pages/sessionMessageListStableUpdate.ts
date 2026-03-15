import type { AutoscrollToBottom, VirtuosoMessageListMethods } from "@virtuoso.dev/message-list";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";

type MessageListMethods = VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext>;

export type StableListRemeasureSpan = {
  start: number;
  count: number;
};

export type StableListUpdateResult = {
  mode: "map" | "remeasure";
  changedSpans: StableListRemeasureSpan[];
};

type StableListUpdateParams = {
  methods: MessageListMethods;
  current: WorkbenchListItem[];
  next: WorkbenchListItem[];
  stickToBottom: boolean;
  anchorIndex: number;
  appendBehavior: AutoscrollToBottom<WorkbenchListItem, WorkbenchMessageListContext>;
  allowAnchorMap?: boolean;
};

export const hashString = (value: string): string => {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
};

const textLayoutKey = (value: string | null | undefined): string => {
  const text = String(value ?? "");
  let lines = 1;
  for (let index = 0; index < text.length; index += 1) {
    if (text.charCodeAt(index) === 10) lines += 1;
  }
  return `${text.length}:${lines}:${hashString(text)}`;
};

const serializeLayoutValue = (value: unknown, seen = new WeakSet<object>()): string => {
  if (value == null) return "null";
  if (typeof value === "string") return `s:${textLayoutKey(value)}`;
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return `${typeof value}:${String(value)}`;
  }
  if (typeof value === "undefined") return "undefined";
  if (typeof value !== "object") return typeof value;
  if (Array.isArray(value)) return `[${value.map((entry) => serializeLayoutValue(entry, seen)).join(",")}]`;
  if (seen.has(value)) return "[circular]";
  seen.add(value);
  const record = value as Record<string, unknown>;
  const serialized = `{${Object.keys(record)
    .sort()
    .map((key) => `${key}:${serializeLayoutValue(record[key], seen)}`)
    .join(",")}}`;
  seen.delete(value);
  return serialized;
};

export const getWorkbenchListItemLayoutKey = (item: WorkbenchListItem): string => {
  switch (item.kind) {
    case "message":
      return `message:${textLayoutKey(item.content)}:${item.attachments.length}`;
    case "assistant":
      return `assistant:${textLayoutKey(item.content)}:${item.is_complete ? 1 : 0}`;
    case "thought":
      return `thought:${textLayoutKey(item.content)}`;
    case "turn_header":
      return `turn_header:${textLayoutKey(item.header.plain_text ?? item.header.content)}:${item.header.attachments.length}`;
    case "tool":
      return [
        "tool",
        textLayoutKey(item.title),
        textLayoutKey(item.subtitle),
        textLayoutKey(item.output_text),
        `kind:${item.tool_kind}`,
        `status:${item.status}`,
        `locations:${serializeLayoutValue(item.locations)}`,
        `input:${serializeLayoutValue(item.input)}`,
        `details:${item.has_details ? 1 : 0}`,
      ].join("|");
    case "tool_group":
      return [
        "tool_group",
        `totals:${item.tool_total}:${item.tool_pending}:${item.tool_running}:${item.tool_completed}:${item.tool_failed}`,
        textLayoutKey(item.thought),
        item.tools.map((tool) => getWorkbenchListItemLayoutKey(tool)).join("||"),
      ].join("|");
    case "turn_status":
      return [
        "turn_status",
        `status:${item.status}`,
        textLayoutKey(item.custom_status),
        `copy:${(item.assistant_messages_content ?? "").trim().length > 0 ? 1 : 0}`,
      ].join("|");
    case "ask_user_question":
      return [
        "ask_user_question",
        serializeLayoutValue(item.input),
        serializeLayoutValue(item.answers ?? null),
        `answered:${item.answered ? 1 : 0}`,
        `outcome:${item.outcome ?? ""}`,
      ].join("|");
    case "spacer":
      return "spacer";
    default:
      return "unknown";
  }
};

const getContextLayoutRevisionKey = (
  item: WorkbenchListItem,
  context: WorkbenchMessageListContext | undefined,
): string => {
  const renderRevision = context?.renderRevision;
  if (!renderRevision) return "";
  switch (item.kind) {
    case "tool":
    case "tool_group":
    case "turn_header":
      return hashString(renderRevision);
    default:
      return "";
  }
};

export const getWorkbenchListItemRenderKey = (
  item: WorkbenchListItem,
  context?: WorkbenchMessageListContext,
): string => {
  const itemLayoutHash = hashString(getWorkbenchListItemLayoutKey(item));
  const contextLayoutHash = getContextLayoutRevisionKey(item, context);
  return contextLayoutHash.length > 0 ? `${item.id}:${itemLayoutHash}:${contextLayoutHash}` : `${item.id}:${itemLayoutHash}`;
};

export const findStableListRemeasureSpans = (
  current: WorkbenchListItem[],
  next: WorkbenchListItem[],
): StableListRemeasureSpan[] => {
  const spans: StableListRemeasureSpan[] = [];
  let spanStart = -1;

  for (let index = 0; index < current.length; index += 1) {
    const currentItem = current[index];
    const nextItem = next[index];
    const changed =
      currentItem !== nextItem &&
      (currentItem?.id !== nextItem?.id ||
        getWorkbenchListItemLayoutKey(currentItem!) !== getWorkbenchListItemLayoutKey(nextItem!));

    if (changed) {
      if (spanStart < 0) spanStart = index;
      continue;
    }
    if (spanStart >= 0) {
      spans.push({ start: spanStart, count: index - spanStart });
      spanStart = -1;
    }
  }

  if (spanStart >= 0) {
    spans.push({ start: spanStart, count: current.length - spanStart });
  }

  return spans;
};

const applyMappedUpdate = ({
  methods,
  nextById,
  stickToBottom,
  anchorIndex,
  allowAnchorMap = true,
}: {
  methods: MessageListMethods;
  nextById: ReadonlyMap<string, WorkbenchListItem>;
  stickToBottom: boolean;
  anchorIndex: number;
  allowAnchorMap?: boolean;
}) => {
  if (allowAnchorMap && !stickToBottom && anchorIndex >= 0) {
    methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
    return;
  }
  methods.data.map(
    (item) => nextById.get(item.id) ?? item,
    stickToBottom ? ("auto" as const) : undefined,
  );
};

export function applyStableListUpdate({
  methods,
  current,
  next,
  prefix = [],
  suffix = [],
  stickToBottom,
  anchorIndex,
  appendBehavior,
  allowAnchorMap = true,
}: StableListUpdateParams & {
  prefix?: WorkbenchListItem[];
  suffix?: WorkbenchListItem[];
}): StableListUpdateResult {
  const nextById = new Map([...prefix, ...next, ...suffix].map((item) => [item.id, item] as const));

  const changedSpans = findStableListRemeasureSpans(current, next);
  const hasEdgeInserts = prefix.length > 0 || suffix.length > 0;
  if (!hasEdgeInserts && changedSpans.length === 0) {
    applyMappedUpdate({
      methods,
      nextById,
      stickToBottom,
      anchorIndex,
      allowAnchorMap,
    });
    return { mode: "map", changedSpans };
  }

  methods.data.batch(
    () => {
      if (prefix.length > 0) methods.data.prepend(prefix);
      if (suffix.length > 0) methods.data.append(suffix, appendBehavior);
      applyMappedUpdate({
        methods,
        nextById,
        stickToBottom,
        anchorIndex,
        allowAnchorMap,
      });
    },
    appendBehavior,
  );

  return { mode: changedSpans.length > 0 ? "remeasure" : "map", changedSpans };
}

export function applyStructuralStableListUpdate({
  methods,
  current,
  next,
  prefixLen,
  suffixLen,
  stickToBottom,
  anchorIndex,
  appendBehavior,
  allowAnchorMap = true,
}: StableListUpdateParams & {
  prefixLen: number;
  suffixLen: number;
}): StableListUpdateResult {
  const deleteCount = current.length - prefixLen - suffixLen;
  const insertData = next.slice(prefixLen, next.length - suffixLen);
  const postStructureCurrent = [
    ...current.slice(0, prefixLen),
    ...insertData,
    ...current.slice(current.length - suffixLen),
  ];
  const changedSpans = findStableListRemeasureSpans(postStructureCurrent, next);
  const nextById = new Map(next.map((item) => [item.id, item] as const));

  methods.data.batch(
    () => {
      if (deleteCount > 0) methods.data.deleteRange(prefixLen, deleteCount);
      if (insertData.length > 0) methods.data.insert(insertData, prefixLen, appendBehavior);
      applyMappedUpdate({
        methods,
        nextById,
        stickToBottom,
        anchorIndex,
        allowAnchorMap,
      });
    },
    appendBehavior,
  );

  return { mode: changedSpans.length > 0 ? "remeasure" : "map", changedSpans };
}
