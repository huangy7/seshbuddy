import type { ChatMessage, ContentPart } from "../types/session";
import { scanToolPart } from "./toolPartScan";

export interface MessageVisibilityFilters {
  user: boolean;
  assistant: boolean;
  tool: boolean;
  thinking: boolean;
}

export function isWidgetPart(part: ContentPart): boolean {
  if (part.type === "tool_use") {
    return part.tool_name === "show_widget" || scanToolPart(part).hasWidget;
  }
  if (part.type === "tool_result") {
    const scan = scanToolPart(part);
    if (part.is_error || scan.isVisualizerAck) {
      return false;
    }
    return scan.hasWidget;
  }
  return false;
}

function isToolPart(part: ContentPart): boolean {
  if (isWidgetPart(part)) return false;
  return part.type === "tool_use" || part.type === "tool_result";
}

function isThinkingPart(part: ContentPart): boolean {
  return part.type === "thinking";
}

function hasNonToolPart(parts: ContentPart[]): boolean {
  return parts.some((part) => !isToolPart(part));
}

function isToolOnlyMessage(parts: ContentPart[]): boolean {
  return parts.length > 0 && parts.every(isToolPart);
}

/** 该 part 在当前过滤下是否可见：工具与思考由开关控制，图表部件作为助手视觉内容始终跟随助手可见，其余（文本/图片）始终可见 */
function isPartVisible(part: ContentPart, filters: MessageVisibilityFilters): boolean {
  if (isWidgetPart(part)) return true;
  if (isToolPart(part)) return filters.tool;
  if (isThinkingPart(part)) return filters.thinking;
  return true;
}

export function isMessageVisible(msg: ChatMessage, filters: MessageVisibilityFilters): boolean {
  const parts = msg.content_parts;

  if (msg.role === "user") {
    if (isToolOnlyMessage(parts)) {
      return filters.tool;
    }
    if (!filters.user) {
      return false;
    }
    if (!filters.tool) {
      return hasNonToolPart(parts);
    }
    return true;
  }

  if (msg.role === "assistant") {
    if (!filters.assistant) {
      // 助手整体隐藏：仅当消息不含任何正文（文本/图片）且至少一个 part 可见时才保留
      const hasBody = parts.some((part) => !isToolPart(part) && !isThinkingPart(part));
      if (hasBody) return false;
      return parts.some((part) => isPartVisible(part, filters));
    }
    return parts.some((part) => isPartVisible(part, filters));
  }

  return true;
}
