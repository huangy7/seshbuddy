/**
 * SSE 响应体结构化重建：把 Anthropic / OpenAI / Codex responses 的事件流
 * 还原成可读的消息分块（text / thinking / tool_use）+ token usage。
 * 解析失败容忍：无法识别的事件直接跳过，绝不抛出。
 */

export interface SseBlock {
  type: "text" | "thinking" | "tool_use";
  text?: string;
  name?: string;
  /** tool_use 累积的原始 input JSON 字符串（可能不完整，展示时尝试美化） */
  inputJson?: string;
  /** Anthropic tool_use 的 block id / OpenAI tool_call id */
  id?: string;
  /** Anthropic thinking 块的 signature */
  signature?: string;
}

export interface SseUsage {
  input?: number;
  output?: number;
  cacheRead?: number;
  cacheCreation?: number;
}

export type SseFormat = "anthropic" | "openai" | "responses";

export interface ReconstructedSse {
  /** 流格式（决定 assembleSseMessage 的输出结构） */
  format?: SseFormat;
  /** Anthropic message id */
  id?: string;
  model?: string;
  blocks: SseBlock[];
  stopReason?: string | null;
  /** OpenAI finish_reason */
  finishReason?: string | null;
  usage?: SseUsage;
  /** 各家原始 usage 对象（组装最终消息时原样带出） */
  rawUsage?: JsonObject;
}

/** 判断响应体是否为 SSE 流（而非普通 JSON） */
export function isSseBody(raw: string | null | undefined): boolean {
  if (!raw) return false;
  const t = raw.trimStart();
  return t.startsWith("event:") || t.startsWith("data:");
}

interface JsonObject {
  [key: string]: unknown;
}

function asObject(v: unknown): JsonObject | null {
  return v !== null && typeof v === "object" && !Array.isArray(v)
    ? (v as JsonObject)
    : null;
}

function num(v: unknown): number | undefined {
  return typeof v === "number" ? v : undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

/** 从一行 `data: {...}` 解析 JSON，失败返回 null */
function parseDataLine(line: string): JsonObject | null {
  if (!line.startsWith("data:")) return null;
  const jsonStr = line.slice(5).trim();
  if (!jsonStr || jsonStr === "[DONE]") return null;
  try {
    return asObject(JSON.parse(jsonStr));
  } catch {
    return null;
  }
}

export function reconstructSse(raw: string): ReconstructedSse {
  const result: ReconstructedSse = { blocks: [] };
  const usage: SseUsage = {};
  let hasUsage = false;
  const rawUsage: JsonObject = {};

  // Anthropic 按 index 管理 content block；OpenAI tool_calls 也按 index 累积
  const blocksByIndex = new Map<number, SseBlock>();
  let currentIndex = -1;

  const pushBlock = (block: SseBlock): SseBlock => {
    result.blocks.push(block);
    return block;
  };

  for (const line of raw.split("\n")) {
    const data = parseDataLine(line);
    if (!data) continue;
    const type = str(data.type) ?? "";

    // ─── Anthropic Messages API ───
    if (type === "message_start") {
      result.format = result.format ?? "anthropic";
      const msg = asObject(data.message);
      result.model = str(msg?.model) ?? result.model;
      result.id = str(msg?.id) ?? result.id;
      const u = asObject(msg?.usage);
      if (u) {
        Object.assign(rawUsage, u);
        usage.input = num(u.input_tokens) ?? usage.input;
        usage.cacheRead = num(u.cache_read_input_tokens) ?? usage.cacheRead;
        usage.cacheCreation = num(u.cache_creation_input_tokens) ?? usage.cacheCreation;
        hasUsage = true;
      }
      continue;
    }
    if (type === "content_block_start") {
      const idx = num(data.index) ?? ++currentIndex;
      currentIndex = idx;
      const cb = asObject(data.content_block);
      const cbType = str(cb?.type);
      let block: SseBlock;
      if (cbType === "thinking") {
        block = {
          type: "thinking",
          text: str(cb?.thinking) ?? "",
          signature: str(cb?.signature),
        };
      } else if (cbType === "tool_use") {
        block = { type: "tool_use", name: str(cb?.name), inputJson: "", id: str(cb?.id) };
      } else {
        block = { type: "text", text: str(cb?.text) ?? "" };
      }
      pushBlock(block);
      blocksByIndex.set(idx, block);
      continue;
    }
    if (type === "content_block_delta") {
      const idx = num(data.index) ?? currentIndex;
      const block = blocksByIndex.get(idx);
      const delta = asObject(data.delta);
      const deltaType = str(delta?.type);
      if (block && delta) {
        if (deltaType === "text_delta") {
          block.text = (block.text ?? "") + (str(delta.text) ?? "");
        } else if (deltaType === "thinking_delta") {
          block.text = (block.text ?? "") + (str(delta.thinking) ?? "");
        } else if (deltaType === "input_json_delta") {
          block.inputJson = (block.inputJson ?? "") + (str(delta.partial_json) ?? "");
        } else if (deltaType === "signature_delta") {
          block.signature = (block.signature ?? "") + (str(delta.signature) ?? "");
        }
      }
      continue;
    }
    if (type === "message_delta") {
      const delta = asObject(data.delta);
      result.stopReason = str(delta?.stop_reason) ?? result.stopReason;
      const u = asObject(data.usage);
      if (u) {
        Object.assign(rawUsage, u);
        usage.output = num(u.output_tokens) ?? usage.output;
        hasUsage = true;
      }
      continue;
    }

    // ─── OpenAI chat/completions 流式块 ───
    const choices = Array.isArray(data.choices) ? data.choices : null;
    if (choices) {
      result.format = result.format ?? "openai";
      for (const rawChoice of choices) {
        const choice = asObject(rawChoice);
        result.finishReason = str(choice?.finish_reason) ?? result.finishReason;
        const delta = asObject(choice?.delta);
        if (!delta) continue;
        const content = str(delta.content);
        if (content) {
          let last = result.blocks[result.blocks.length - 1];
          if (!last || last.type !== "text") last = pushBlock({ type: "text", text: "" });
          last.text = (last.text ?? "") + content;
        }
        const reasoning = str(delta.reasoning_content);
        if (reasoning) {
          let last = result.blocks[result.blocks.length - 1];
          if (!last || last.type !== "thinking") last = pushBlock({ type: "thinking", text: "" });
          last.text = (last.text ?? "") + reasoning;
        }
        const toolCalls = Array.isArray(delta.tool_calls) ? delta.tool_calls : null;
        if (toolCalls) {
          for (const rawTc of toolCalls) {
            const tc = asObject(rawTc);
            const fn = asObject(tc?.function);
            const idx = num(tc?.index) ?? 0;
            let block = blocksByIndex.get(1000 + idx);
            if (!block) {
              block = { type: "tool_use", name: str(fn?.name), inputJson: "", id: str(tc?.id) };
              pushBlock(block);
              blocksByIndex.set(1000 + idx, block);
            }
            if (str(fn?.name)) block.name = str(fn?.name);
            if (str(tc?.id)) block.id = str(tc?.id);
            block.inputJson = (block.inputJson ?? "") + (str(fn?.arguments) ?? "");
          }
        }
      }
      const u = asObject(data.usage);
      if (u) {
        Object.assign(rawUsage, u);
        usage.input = num(u.prompt_tokens) ?? usage.input;
        usage.output = num(u.completion_tokens) ?? usage.output;
        hasUsage = true;
      }
      continue;
    }

    // ─── Codex responses API ───
    if (type === "response.output_text.delta") {
      result.format = result.format ?? "responses";
      let last = result.blocks[result.blocks.length - 1];
      if (!last || last.type !== "text") last = pushBlock({ type: "text", text: "" });
      last.text = (last.text ?? "") + (str(data.delta) ?? "");
      continue;
    }
    if (type === "response.reasoning_summary_text.delta") {
      result.format = result.format ?? "responses";
      let last = result.blocks[result.blocks.length - 1];
      if (!last || last.type !== "thinking") last = pushBlock({ type: "thinking", text: "" });
      last.text = (last.text ?? "") + (str(data.delta) ?? "");
      continue;
    }
    if (type === "response.output_item.added") {
      result.format = result.format ?? "responses";
      const item = asObject(data.item);
      if (str(item?.type) === "function_call") {
        pushBlock({ type: "tool_use", name: str(item?.name), inputJson: "", id: str(item?.call_id ?? item?.id) });
      }
      continue;
    }
    if (type === "response.function_call_arguments.delta") {
      const last = [...result.blocks].reverse().find((b) => b.type === "tool_use");
      if (last) last.inputJson = (last.inputJson ?? "") + (str(data.delta) ?? "");
      continue;
    }
    if (type === "response.completed") {
      result.format = result.format ?? "responses";
      const u = asObject(asObject(data.response)?.usage);
      if (u) {
        Object.assign(rawUsage, u);
        usage.input = num(u.input_tokens) ?? usage.input;
        usage.output = num(u.output_tokens) ?? usage.output;
        usage.cacheRead = num(asObject(u.input_tokens_details)?.cached_tokens) ?? usage.cacheRead;
        hasUsage = true;
      }
      continue;
    }
  }

  // 清理完全空的分块
  result.blocks = result.blocks.filter(
    (b) => (b.text ?? "") !== "" || (b.inputJson ?? "") !== "" || b.name,
  );
  if (hasUsage) {
    result.usage = usage;
    result.rawUsage = rawUsage;
  }
  return result;
}

/** 尝试把 tool_use 累积的 input JSON 美化；不完整时原样返回 */
export function prettyToolInput(inputJson: string | undefined): string {
  if (!inputJson) return "";
  try {
    return JSON.stringify(JSON.parse(inputJson), null, 2);
  } catch {
    return inputJson;
  }
}

function parseJsonOrRaw(raw: string | undefined): unknown {
  if (!raw) return {};
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}

/**
 * 把 SSE 流组装成「非流式响应同构」的 JSON 对象 —— 即 stream=false 时
 * HTTP 响应体本该长成的样子，与 Request Body 对称展示。
 * 无法组装（无有效分块）时返回 null，调用方回退展示原始流。
 */
export function assembleSseMessage(raw: string): JsonObject | null {
  const r = reconstructSse(raw);
  if (r.blocks.length === 0) return null;

  if (r.format === "openai") {
    const text = r.blocks.filter((b) => b.type === "text").map((b) => b.text ?? "").join("");
    const reasoning = r.blocks.filter((b) => b.type === "thinking").map((b) => b.text ?? "").join("");
    const toolCalls = r.blocks
      .filter((b) => b.type === "tool_use")
      .map((b, i) => ({
        id: b.id ?? `call_${i}`,
        type: "function",
        function: { name: b.name ?? "", arguments: b.inputJson ?? "" },
      }));
    const message: JsonObject = { role: "assistant", content: text || null };
    if (reasoning) message.reasoning_content = reasoning;
    if (toolCalls.length > 0) message.tool_calls = toolCalls;
    return {
      object: "chat.completion",
      ...(r.model ? { model: r.model } : {}),
      choices: [{ index: 0, message, finish_reason: r.finishReason ?? null }],
      ...(r.rawUsage ? { usage: r.rawUsage } : {}),
    };
  }

  if (r.format === "responses") {
    const output = r.blocks.map((b) => {
      if (b.type === "tool_use") {
        return {
          type: "function_call",
          name: b.name ?? "",
          arguments: b.inputJson ?? "",
          ...(b.id ? { call_id: b.id } : {}),
        };
      }
      if (b.type === "thinking") {
        return { type: "reasoning", summary: [{ type: "summary_text", text: b.text ?? "" }] };
      }
      return { type: "message", role: "assistant", content: [{ type: "output_text", text: b.text ?? "" }] };
    });
    return {
      object: "response",
      status: "completed",
      ...(r.model ? { model: r.model } : {}),
      output,
      ...(r.rawUsage ? { usage: r.rawUsage } : {}),
    };
  }

  // 默认 Anthropic Messages 结构
  const content = r.blocks.map((b) => {
    if (b.type === "thinking") {
      return {
        type: "thinking",
        thinking: b.text ?? "",
        ...(b.signature ? { signature: b.signature } : {}),
      };
    }
    if (b.type === "tool_use") {
      return {
        type: "tool_use",
        ...(b.id ? { id: b.id } : {}),
        name: b.name ?? "",
        input: parseJsonOrRaw(b.inputJson),
      };
    }
    return { type: "text", text: b.text ?? "" };
  });
  return {
    ...(r.id ? { id: r.id } : {}),
    type: "message",
    role: "assistant",
    ...(r.model ? { model: r.model } : {}),
    content,
    stop_reason: r.stopReason ?? null,
    stop_sequence: null,
    ...(r.rawUsage ? { usage: r.rawUsage } : {}),
  };
}
