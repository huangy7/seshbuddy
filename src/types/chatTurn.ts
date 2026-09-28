import type { ChatMessage, TokenUsage } from "./session";

export interface ToolBadgeItem {
  id: string;
  tool_name: string;
  summary: string;
  input: string;
  content?: string;
  is_error: boolean;
  tool_use_id?: string;
  originalIndex: number;
  partIndex?: number;
  /** 来源 part 类型（tool_result 的懒加载字段仅在该类型下有值） */
  part_type?: "tool_use" | "tool_result";
  /** true = input 只是预览，全文按 source_offset 调 get_tool_result_full_content 拉取 */
  truncated_preview?: boolean;
  /** true = input 在 50 KB 处被截断，渲染时追加本地化的截断提示 */
  truncated?: boolean;
  /** 截断后全文长度（≤ 50,000），供「加载全文」提示展示 */
  full_len?: number;
  /** 该行在会话文件中的字节 offset（懒加载定位符） */
  source_offset?: number;
}

export interface ToolBadgeGroup {
  tool_name: string;
  count: number;
  hasError: boolean;
  items: ToolBadgeItem[];
}

export interface ToolGroupSegment {
  type: 'tool_group';
  groups: ToolBadgeGroup[];
  hasError: boolean;
}

export interface TextSegment {
  type: 'text';
  /** 该段合并后的完整正文（复制 / Fork 取此值） */
  text: string;
  originalIndex: number;
  /** 该段首个 text part 在原始消息 content_parts 内的下标（打包消息如 DSH
      [thinking, text, tool_use] 时非 0；渲染缓存按 (originalIndex, partIndex) 回查） */
  partIndex: number;
  /** 合并进该段正文的源码消息索引（流式分片会横跨多条） */
  indexes: number[];
  /**
   * 合并进该段的各 text part（`index` 为在 content_parts 内的真实下标）。
   *
   * 渲染侧必须逐 part 回查渲染缓存，不能只按 `partIndex` 取首段 ——
   * 同一消息内连续多段文本会被合并成一个气泡，若只渲染首段，后面的正文会静默丢失
   * （复制/Fork 走 `text` 却带着全部内容，两者还会不一致）。
   * 合并只发生在**连续**的 text part 之间，因此这些下标必然连续。
   */
  textParts: Array<{ index: number; text: string }>;
}

export interface ThinkingSegment {
  type: 'thinking';
  thinking: string;
  originalIndex: number;
  indexes: number[];
}

export interface WidgetSegment {
  type: 'widget';
  widgetType: 'svg' | 'html';
  title: string;
  code: string;
  originalIndex: number;
  partIndex?: number;
}

export type AssistantSegment = TextSegment | ThinkingSegment | ToolGroupSegment | WidgetSegment;

export interface TurnNode {
  id: string;
  userMessage?: {
    msg: ChatMessage;
    originalIndex: number;
  };
  assistantTurn?: {
    model: string | null;
    timestamp: string;
    token_usage?: TokenUsage | null;
    segments: AssistantSegment[];
    originalIndexes: number[];
  };
}
