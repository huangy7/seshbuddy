/**
 * 工具类 part 的载荷扫描 —— **全应用唯一允许整串读取工具载荷的地方**。
 *
 * 背景：工具结果正文单条上限 50 KB（后端 `parser/claude.rs` 的截断），而"是不是图表部件"
 * "是不是错误""是不是图表回执"这些判定都必须读一遍正文。大会话在流式加载过程中会对同一条
 * 消息反复判定，若各处各自实现，既会重复扫描，行为也会漂移（历史上 `messageFilter` 与
 * `useMessageTurns` 就各自维护了一份近似但不同的判定）。
 *
 * 因此：
 * 1. 所有针对工具载荷的整串扫描收敛到本模块，调用方只消费结论；
 * 2. 结论按 part 对象用 `WeakMap` 记忆化 —— part 在会话生命周期内是**不可变引用**
 *    （`useStreamingLoad` 只 append 新对象；`ChatView.loadIncremental` 用
 *    `[...messages.value, ...result.messages]` 重建数组），所以缓存不会读到陈旧值，
 *    旧键随对象一起被 GC，无需（也不应）手动清理。
 *
 * 判定策略仍留在各调用方：本模块只回答"载荷里有什么"，不回答"这算不算可见/算不算图表"。
 */
import type { ContentPart } from "../types/session";
import { extractWidgetData, type WidgetData } from "./svg";

/** 图表部件回执标记：工具结果正文带此串表示它是图表调用的 ACK，不应再当部件渲染 */
export const VISUALIZER_ACK_MARKER = "visualizer_show_widget_result";
/** 部分 CLI 只在正文里表达失败，摘要里看不出来 */
export const TOOL_ERROR_MARKER = "[Tool Error]";

export interface ToolPartScan {
  /** 载荷中提取到了图表数据 */
  hasWidget: boolean;
  /** 提取结果；`hasWidget` 为 false 时为 null */
  widgetData: WidgetData | null;
  /** 仅 tool_result：正文带 {@link VISUALIZER_ACK_MARKER} */
  isVisualizerAck: boolean;
  /** 仅 tool_result：正文带 {@link TOOL_ERROR_MARKER} */
  hasToolErrorMarker: boolean;
}

const EMPTY_SCAN: ToolPartScan = Object.freeze({
  hasWidget: false,
  widgetData: null,
  isVisualizerAck: false,
  hasToolErrorMarker: false,
});

const scanCache = new WeakMap<ContentPart, ToolPartScan>();

/**
 * 扫描工具类 part 的载荷（结果按 part 记忆化，重复调用零成本）。
 * 非 `tool_use` / `tool_result` 的 part 直接返回空结论，不占用缓存。
 */
export function scanToolPart(part: ContentPart): ToolPartScan {
  if (part.type !== "tool_use" && part.type !== "tool_result") return EMPTY_SCAN;

  const cached = scanCache.get(part);
  if (cached) return cached;

  const payload = part.type === "tool_use" ? part.input : part.content;
  const widgetData = extractWidgetData(payload);
  const scan: ToolPartScan = Object.freeze({
    hasWidget: widgetData !== null,
    widgetData,
    isVisualizerAck:
      part.type === "tool_result" &&
      Boolean(part.content && part.content.includes(VISUALIZER_ACK_MARKER)),
    hasToolErrorMarker:
      part.type === "tool_result" &&
      Boolean(part.content && part.content.includes(TOOL_ERROR_MARKER)),
  });

  scanCache.set(part, scan);
  return scan;
}
