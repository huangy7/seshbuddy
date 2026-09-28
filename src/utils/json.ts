/**
 * JSON syntax highlighting utilities.
 * Produces HTML with class names: json-key, json-str, json-num, json-bool, json-null.
 * Consumers must provide matching CSS (or use :deep() in scoped styles).
 */
import { t } from "../i18n";

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function renderJson(value: unknown, indent: number): string {
  const pad = "  ".repeat(indent);
  const padInner = "  ".repeat(indent + 1);

  if (value === null) return '<span class="json-null">null</span>';
  if (typeof value === "boolean") return `<span class="json-bool">${value}</span>`;
  if (typeof value === "number") return `<span class="json-num">${value}</span>`;
  if (typeof value === "string") {
    const escaped = escapeHtml(value);
    if (escaped.length > 2000) {
      return `<span class="json-str">"${escaped.slice(0, 2000)}..."</span>`;
    }
    return `<span class="json-str">"${escaped}"</span>`;
  }

  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    const items = value.map((v) => `${padInner}${renderJson(v, indent + 1)}`);
    return `[\n${items.join(",\n")}\n${pad}]`;
  }

  if (typeof value === "object" && value !== null) {
    const keys = Object.keys(value);
    if (keys.length === 0) return "{}";
    const entries = keys.map((k) => {
      const keyHtml = `<span class="json-key">"${escapeHtml(k)}"</span>`;
      return `${padInner}${keyHtml}: ${renderJson((value as Record<string, unknown>)[k], indent + 1)}`;
    });
    return `{\n${entries.join(",\n")}\n${pad}}`;
  }

  return String(value);
}

/**
 * Parse a JSON string and return syntax-highlighted HTML.
 * Falls back to escaped plain text if parsing fails.
 */
export function highlightJson(raw: string | null): string {
  if (!raw) return `<span class="json-null">${t("common.jsonViewer.empty")}</span>`;
  let obj: unknown;
  try {
    obj = JSON.parse(raw);
  } catch {
    return escapeHtml(raw);
  }
  return renderJson(obj, 0);
}

/**
 * Parse Claude SSE response body and aggregate into a structured JSON object.
 * Combines text deltas, tool_use blocks, usage info, and stop_reason.
 * Returns syntax-highlighted HTML.
 */
export function highlightSseResponse(raw: string | null): string {
  // 空值提示走语言包：闸门规则 1 只认 CJK，`(empty)` 这类纯英文界面文案它看不见。
  // 取值必须写在函数体里——模块级求值会让它在 import 时被定死，切换语言后不再跟随（规则 7）。
  if (!raw) return `<span class="json-null">${t("common.jsonViewer.empty")}</span>`;

  // Quick check: if it starts with '{', it's regular JSON, not SSE
  const trimmed = raw.trimStart();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    return highlightJson(raw);
  }

  // Parse SSE events
  const aggregated = aggregateSseEvents(raw);
  return renderJson(aggregated, 0);
}

interface SseContentBlock {
  type: string;
  text?: string;
  tool_name?: string;
  tool_input?: string;
  id?: string;
}

function aggregateSseEvents(raw: string): Record<string, unknown> {
  const lines = raw.split("\n");
  let model = "";
  let messageId = "";
  let stopReason: string | null = null;
  const usage: Record<string, unknown> = {};
  const contentBlocks: SseContentBlock[] = [];
  let currentBlockIndex = -1;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line.startsWith("data:")) continue;
    const jsonStr = line.slice(5).trim();
    if (!jsonStr) continue;

    let data: Record<string, unknown>;
    try {
      const parsed = JSON.parse(jsonStr);
      if (parsed === null || typeof parsed !== "object") continue;
      data = parsed as Record<string, unknown>;
    } catch {
      continue;
    }

    const eventType = (data as Record<string, unknown>).type as string;

    if (eventType === "message_start") {
      const msg = data.message as Record<string, unknown> | undefined;
      if (msg) {
        model = (msg.model as string) || "";
        messageId = (msg.id as string) || "";
        if (msg.usage) Object.assign(usage, msg.usage);
      }
    } else if (eventType === "content_block_start") {
      const block = data.content_block as Record<string, unknown> | undefined;
      const idx = (data.index as number) ?? contentBlocks.length;
      currentBlockIndex = idx;
      if (block) {
        const blockType = block.type as string;
        if (blockType === "text") {
          contentBlocks[idx] = { type: "text", text: "" };
        } else if (blockType === "tool_use") {
          contentBlocks[idx] = {
            type: "tool_use",
            tool_name: (block.name as string) || "",
            id: (block.id as string) || "",
            tool_input: "",
          };
        } else {
          contentBlocks[idx] = { type: blockType };
        }
      }
    } else if (eventType === "content_block_delta") {
      const idx = (data.index as number) ?? currentBlockIndex;
      const delta = data.delta as Record<string, unknown> | undefined;
      if (delta && contentBlocks[idx]) {
        const deltaType = delta.type as string;
        if (deltaType === "text_delta" && contentBlocks[idx].type === "text") {
          contentBlocks[idx].text = (contentBlocks[idx].text || "") + (delta.text as string || "");
        } else if (deltaType === "input_json_delta" && contentBlocks[idx].type === "tool_use") {
          contentBlocks[idx].tool_input = (contentBlocks[idx].tool_input || "") + (delta.partial_json as string || "");
        }
      }
    } else if (eventType === "message_delta") {
      const delta = data.delta as Record<string, unknown> | undefined;
      if (delta?.stop_reason) stopReason = delta.stop_reason as string;
      if (data.usage) Object.assign(usage, data.usage);
    } else if (eventType === "message_stop") {
      const metrics = data["amazon-bedrock-invocationMetrics"] as Record<string, unknown> | undefined;
      if (metrics) {
        usage["invocation_metrics"] = metrics;
      }
    }
  }

  // Build aggregated result
  const content: unknown[] = contentBlocks.map((block) => {
    if (block.type === "text") {
      return { type: "text", text: block.text || "" };
    } else if (block.type === "tool_use") {
      let parsedInput: unknown = block.tool_input || "";
      try {
        parsedInput = JSON.parse(block.tool_input || "{}");
      } catch { /* keep as string */ }
      return { type: "tool_use", name: block.tool_name, id: block.id, input: parsedInput };
    }
    return { type: block.type };
  });

  const result: Record<string, unknown> = {};
  if (messageId) result.id = messageId;
  if (model) result.model = model;
  if (stopReason) result.stop_reason = stopReason;
  if (content.length > 0) result.content = content;
  if (Object.keys(usage).length > 0) result.usage = usage;

  return result;
}
