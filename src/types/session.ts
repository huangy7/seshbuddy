import type { CliId } from "./cli";

export interface SessionIdentity {
  cliId: CliId;
  filePath: string;
}

/**
 * 导出成功后的结果。**不是错误**——后端不再造句子，前端按 `kind` 渲染本地化提示。
 *
 * `count` 只出现在真正用到它的变体上：`single` 的会话数恒为 1，
 * 给它一个 `count` 只会让渲染层插一个假数字。
 */
export type ExportOutcome =
  | { kind: "single" }
  | { kind: "merged"; count: number }
  | { kind: "separate"; count: number };

export function sessionIdentityKey(value: SessionIdentity): string {
  return `${value.cliId}\u0000${value.filePath}`;
}

export interface ProjectInfo {
  /** 项目分组键；`null` = 后端解析不出项目路径（与 `original_path` 同生共死）。 */
  encoded_dir: string | null;
  /** 项目路径；`null` = 没有这个值，展示处用 `t("session.unknownProject")` 兜底。 */
  original_path: string | null;
  sessions: SessionInfo[];
}

export interface AggregatedProjectInfo extends ProjectInfo {
  /** Stable normalized path used to merge the same workspace across CLIs. */
  project_key: string;
  /** CLI sources represented by the currently visible sessions. */
  cli_ids: CliId[];
}

export interface SessionInfo {
  session_id: string;
  file_path: string;
  display_name: string;
  timestamp: string;
  file_size: number;
  git_branch: string;
  has_archive_snapshot: boolean;
  is_archived: boolean;
  cli_id: string;
}

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

export interface ChatMessage {
  role: string;
  timestamp: string;
  model: string | null;
  token_usage?: TokenUsage | null;
  content_parts: ContentPart[];
  is_meta?: boolean;
  /** Transcript entry uuid (Claude & WorkBuddy only) — stable anchor for fork-from-here. */
  uuid?: string;
}

export interface SubagentInfo {
  file_path: string;
  label: string;
}

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "tool_use"; summary: string; tool_name: string; input: string; tool_use_id?: string }
  | {
      type: "tool_result";
      summary: string;
      content: string;
      is_error: boolean;
      tool_use_id?: string;
      /** 截断后全文长度（≤50000），懒加载时供「加载全文」展示 */
      full_len?: number;
      /** true = content 只是预览，全文需按 source_offset 调 get_tool_result_full_content 拉取 */
      truncated_preview?: boolean;
      /** true = content 在 50 KB 处被截断，正文本身不完整（与 truncated_preview 无关） */
      truncated?: boolean;
      /** 该行在会话文件中的字节 offset（append-only 下稳定） */
      source_offset?: number;
    }
  | { type: "thinking"; thinking: string }
  | { type: "image"; media_type: string; data: string }
  | { type: "image_ref"; path: string }
  | { type: "image_meta" };

/**
 * 搜索命中的字段（后端 `MatchedField` 的镜像）。`title` / `first_message` / `session_id`
 * 有对应的展示标签；`content`（正文命中）不渲染前缀。
 * `null` = 后端无从分类，同样不渲染标签。
 *
 * ⚠️ 本联合必须与 `src-tauri/src/session.rs` 的 `MatchedField` 枚举逐一对应：
 * Rust 侧加变体而这里没跟上时，`matchedFieldLabel` 的 `never` 哨兵**不会**报错
 * （哨兵只管 TS 内部的穷举），字面量会被粘上屏。`matchedField.parity.test.ts` 钉住这条。
 */
export type MatchedField = "title" | "first_message" | "session_id" | "content";

export interface SearchResult {
  session_id: string;
  file_path: string;
  display_name: string;
  /** `null` = 后端解析不出项目路径，展示处用 `t("session.unknownProject")` 兜底。 */
  project_path: string | null;
  snippet: string;
  /** 命中字段；`null` = 后端无从分类，没有可展示的字段标签。 */
  matched_field: MatchedField | null;
  match_count: number;
  first_match_message_index: number | null;
  cli_id: string;
}

export interface ContextMenuItem {
  /** 稳定标识：菜单项需要被程序回查（如复制后改写自身文案），而 label 会随语言变化，不能当键 */
  id?: string;
  label: string;
  action?: () => void | unknown | Promise<unknown>;
  separator?: boolean;
  icon?: string;
  danger?: boolean;
  closeOnClick?: boolean;
  children?: ContextMenuItem[];
}

export interface UsageRecord {
  date: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
  duration_ms: number | null;
  project: string;
}

export interface BookmarkInfo {
  cliId: CliId;
  sessionId: string;
  messageIndex: number;
  note?: string;
  createdAt: string;
}

export interface SessionStats {
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_creation_tokens: number;
  total_cache_read_tokens: number;
  total_duration_ms: number;
  turn_count: number;
}

export interface SessionLoadResult {
  messages: ChatMessage[];
  offset: number;
  subagent_map: Record<string, SubagentInfo>;
}
