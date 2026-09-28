/**
 * 解析 LLM 请求体中的 Messages 对话流（支持 Anthropic Messages API / OpenAI Chat Completions API）
 * 包含超大报文（如 1MB+ 复杂会话）的内存缓存与渲染保护机制
 */

import { t, currentLocale } from "../i18n";
import { formatNumber } from "./format";

export interface ParsedMessageContentBlock {
  type: "text" | "image" | "tool_use" | "tool_result" | "thinking" | "unknown";
  text?: string;
  toolName?: string;
  toolCallId?: string;
  toolInput?: Record<string, unknown> | string;
  toolOutput?: string;
  isError?: boolean;
  isTruncated?: boolean;
  fullOutputLength?: number;
}

export interface ParsedMessageItem {
  id: string;
  role: "system" | "user" | "assistant" | "tool";
  name?: string;
  contentBlocks: ParsedMessageContentBlock[];
  rawContent?: string;
}

export interface ParsedRequestPayload {
  model?: string;
  systemPrompt?: string;
  temperature?: number;
  maxTokens?: number;
  messages: ParsedMessageItem[];
  hasMessages: boolean;
}

// 内存解析缓存：避免超大 JSON 响应式更新时反复全量解析
const parsedCache = new Map<string, ParsedRequestPayload>();
const MAX_CACHE_ENTRIES = 20;

/**
 * 缓存键含界面语言：解析结果里烘着 t() 的取值（`imageContentLabel` 与 `truncatedSuffix`），
 * 只按 reqBody 缓存会让换语言后的缓存命中返回旧语言的正文，而同一次输出里的其余标签是
 * 现取 t() 的——结果是一份正文里新旧语言混排。键不含 locale 就没有失效时机。
 *
 * 语言取值是固定枚举、不含 `:`，故 `语言:报文` 不会与另一种语言拼接出的键相撞。
 */
function cacheKeyFor(reqBody: string): string {
  return `${currentLocale.value}:${reqBody}`;
}

const MAX_PREVIEW_TEXT_LENGTH = 32768; // 32KB 安全预览截断

export function parseRequestMessages(reqBody: string | null | undefined): ParsedRequestPayload {
  const emptyResult: ParsedRequestPayload = {
    messages: [],
    hasMessages: false,
  };

  if (!reqBody) return emptyResult;

  // 检查缓存
  if (typeof reqBody === "string") {
    const cached = parsedCache.get(cacheKeyFor(reqBody));
    if (cached) return cached;
  }

  try {
    const json = typeof reqBody === "string" ? JSON.parse(reqBody) : reqBody;
    if (!json || typeof json !== "object") return emptyResult;

    const result: ParsedRequestPayload = {
      messages: [],
      hasMessages: false,
    };

    if (json.model && typeof json.model === "string") {
      result.model = json.model;
    }
    if (json.temperature != null) {
      result.temperature = Number(json.temperature);
    }
    if (json.max_tokens != null || json.max_completion_tokens != null) {
      result.maxTokens = Number(json.max_tokens ?? json.max_completion_tokens);
    }

    // 提取 system prompt
    if (typeof json.system === "string") {
      result.systemPrompt = json.system;
    } else if (Array.isArray(json.system)) {
      result.systemPrompt = json.system
        .map((b: any) => (typeof b === "string" ? b : b?.text ?? ""))
        .filter(Boolean)
        .join("\n\n");
    }

    const rawMsgs = Array.isArray(json.messages) ? json.messages : [];
    if (rawMsgs.length > 0) {
      result.hasMessages = true;
    }

    let msgIdx = 0;
    for (const msg of rawMsgs) {
      msgIdx++;
      const role = (msg?.role ?? "user") as ParsedMessageItem["role"];

      if (role === "system" && !result.systemPrompt && typeof msg?.content === "string") {
        result.systemPrompt = msg.content;
        continue;
      }

      const item: ParsedMessageItem = {
        id: `msg-${msgIdx}`,
        role,
        name: msg?.name,
        contentBlocks: [],
      };

      const content = msg?.content;
      if (typeof content === "string") {
        item.contentBlocks.push({
          type: "text",
          text: content,
        });
        item.rawContent = content;
      } else if (Array.isArray(content)) {
        for (const block of content) {
          if (typeof block === "string") {
            item.contentBlocks.push({ type: "text", text: block });
          } else if (block?.type === "text") {
            item.contentBlocks.push({ type: "text", text: block.text ?? "" });
          } else if (block?.type === "image" || block?.type === "image_url") {
            item.contentBlocks.push({ type: "image", text: t("common.reqMessages.imageContentLabel") });
          } else if (block?.type === "thinking") {
            item.contentBlocks.push({
              type: "thinking",
              text: block.thinking ?? block.text ?? "",
            });
          } else if (block?.type === "tool_use") {
            item.contentBlocks.push({
              type: "tool_use",
              toolName: block.name,
              toolCallId: block.id,
              toolInput: block.input,
            });
          } else if (block?.type === "tool_result") {
            let outputText = "";
            if (typeof block.content === "string") {
              outputText = block.content;
            } else if (Array.isArray(block.content)) {
              outputText = block.content
                .map((c: any) => (typeof c === "string" ? c : c?.text ?? JSON.stringify(c)))
                .join("\n");
            } else if (block.content) {
              outputText = JSON.stringify(block.content, null, 2);
            }

            const fullLen = outputText.length;
            const isTruncated = fullLen > MAX_PREVIEW_TEXT_LENGTH;
            const safeText = isTruncated
              ? outputText.slice(0, MAX_PREVIEW_TEXT_LENGTH) +
                t("common.reqMessages.truncatedSuffix", { count: formatNumber(fullLen) })
              : outputText;

            item.contentBlocks.push({
              type: "tool_result",
              toolCallId: block.tool_use_id,
              toolOutput: safeText,
              isError: Boolean(block.is_error),
              isTruncated,
              fullOutputLength: fullLen,
            });
          } else {
            item.contentBlocks.push({
              type: "unknown",
              text: JSON.stringify(block, null, 2),
            });
          }
        }
      }

      // 兼容 OpenAI function_call / tool_calls
      if (Array.isArray(msg?.tool_calls)) {
        for (const tc of msg.tool_calls) {
          let parsedInput: any = tc.function?.arguments;
          try {
            if (typeof parsedInput === "string") parsedInput = JSON.parse(parsedInput);
          } catch {
            // keep as string
          }
          item.contentBlocks.push({
            type: "tool_use",
            toolName: tc.function?.name,
            toolCallId: tc.id,
            toolInput: parsedInput,
          });
        }
      }

      result.messages.push(item);
    }

    // 写入缓存
    if (typeof reqBody === "string") {
      if (parsedCache.size >= MAX_CACHE_ENTRIES) {
        const firstKey = parsedCache.keys().next().value;
        if (firstKey) parsedCache.delete(firstKey);
      }
      parsedCache.set(cacheKeyFor(reqBody), result);
    }

    return result;
  } catch {
    return emptyResult;
  }
}

/**
 * 将解析出的 Messages 转为适合 Diff 阅读的人类可读 Markdown 对话流文本
 */
export function formatMessagesToReadableText(reqBody: string | null | undefined): string {
  const payload = parseRequestMessages(reqBody);
  const lines: string[] = [];

  if (payload.systemPrompt) {
    lines.push(t("common.reqMessages.systemPromptHeading"));
    lines.push(payload.systemPrompt.trim());
    lines.push("");
  }

  let idx = 0;
  for (const msg of payload.messages) {
    idx++;
    const roleTag = msg.role.toUpperCase();
    const sender = msg.name ? ` (${msg.name})` : "";
    lines.push(`=== #${idx} ${roleTag}${sender} ===`);

    for (const block of msg.contentBlocks) {
      if (block.type === "text" && block.text) {
        lines.push(block.text.trim());
      } else if (block.type === "thinking" && block.text) {
        lines.push(t("common.reqMessages.thinkingLabel"));
        lines.push(block.text.trim());
      } else if (block.type === "tool_use") {
        const toolName = block.toolName ?? t("common.reqMessages.unknownToolName");
        lines.push(t("common.reqMessages.toolUseLabel", { name: toolName }));
        try {
          const inputStr = typeof block.toolInput === "object"
            ? JSON.stringify(block.toolInput, null, 2)
            : String(block.toolInput ?? "");
          lines.push(inputStr);
        } catch {
          lines.push(String(block.toolInput ?? ""));
        }
      } else if (block.type === "tool_result") {
        const errTag = block.isError ? t("common.reqMessages.errorTag") : "";
        lines.push(t("common.reqMessages.toolResultLabel", { tag: errTag }));
        lines.push(block.toolOutput?.trim() ?? "");
      } else if (block.type === "image") {
        lines.push(t("common.reqMessages.imageContentLabel"));
      }
    }
    lines.push("");
  }

  return lines.join("\n").trim();
}

export interface ContextDeltaResult {
  isFirstRequest: boolean;
  systemPromptChanged: boolean;
  oldSystemPrompt?: string;
  newSystemPrompt?: string;
  addedMessages: ParsedMessageItem[];
  prevMessagesCount: number;
  currentMessagesCount: number;
  totalCharsAdded: number;
}

/**
 * 精准对比两轮请求，提取出本轮相比上一轮纯新增的上下文增量
 */
export function computeContextDelta(
  prevReqBody: string | null | undefined,
  currentReqBody: string | null | undefined,
): ContextDeltaResult {
  const current = parseRequestMessages(currentReqBody);

  if (!prevReqBody) {
    let chars = (current.systemPrompt?.length ?? 0);
    for (const m of current.messages) {
      if (m.rawContent) chars += m.rawContent.length;
      for (const b of m.contentBlocks) {
        if (b.text) chars += b.text.length;
        if (b.toolOutput) chars += b.toolOutput.length;
      }
    }
    return {
      isFirstRequest: true,
      systemPromptChanged: false,
      addedMessages: current.messages,
      prevMessagesCount: 0,
      currentMessagesCount: current.messages.length,
      totalCharsAdded: chars,
    };
  }

  const prev = parseRequestMessages(prevReqBody);
  const systemPromptChanged = (prev.systemPrompt ?? "") !== (current.systemPrompt ?? "");

  // 提取新增的消息切片（处理可能存在的增量）
  let addedMessages: ParsedMessageItem[] = [];
  if (current.messages.length > prev.messages.length) {
    addedMessages = current.messages.slice(prev.messages.length);
  } else if (current.messages.length > 0) {
    // 轮数没变但末尾消息内容可能发生了追加
    const lastPrev = prev.messages[prev.messages.length - 1];
    const lastCur = current.messages[current.messages.length - 1];
    if (JSON.stringify(lastPrev) !== JSON.stringify(lastCur)) {
      addedMessages = [lastCur];
    }
  }

  let totalCharsAdded = 0;
  for (const m of addedMessages) {
    if (m.rawContent) totalCharsAdded += m.rawContent.length;
    for (const b of m.contentBlocks) {
      if (b.text) totalCharsAdded += b.text.length;
      if (b.toolOutput) totalCharsAdded += b.toolOutput.length;
    }
  }
  if (systemPromptChanged) {
    totalCharsAdded += Math.abs((current.systemPrompt?.length ?? 0) - (prev.systemPrompt?.length ?? 0));
  }

  return {
    isFirstRequest: false,
    systemPromptChanged,
    oldSystemPrompt: prev.systemPrompt,
    newSystemPrompt: current.systemPrompt,
    addedMessages,
    prevMessagesCount: prev.messages.length,
    currentMessagesCount: current.messages.length,
    totalCharsAdded,
  };
}
