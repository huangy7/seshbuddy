import { reactive } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { t } from "../i18n";
import type { ContentPart } from "../types/session";

/**
 * tool_result 懒加载全文缓存（外挂 Map）。
 *
 * 设计稿《tool-result-lazy-content-design》§4：
 * - 缓存放**外挂 Map**（`${messageIndex}-${partIndex}` → 全文），**不替换 part 对象** ——
 *   `toolPartScan.ts` 的 WeakMap 记忆化依赖 part 引用稳定；
 * - 展开折叠块时自动拉取全文；复制时 await 拉全文再复制；
 * - 拉取失败回退预览，并暴露失败态供界面提示。
 */

type ToolResultPart = Extract<ContentPart, { type: "tool_result" }>;

/** 取全文定位所需的最小字段（ToolBadgeItem 等视图模型也可满足） */
export interface LazyToolResultRef {
  truncated_preview?: boolean;
  truncated?: boolean;
  source_offset?: number;
  tool_use_id?: string;
}

// 全文缓存与在途请求：模块级单例，跨组件共享（气泡视图 / 列表视图指向同一份）
const fullContentCache = reactive(new Map<string, string>());
const failedKeys = reactive(new Set<string>());
const pendingFetches = new Map<string, Promise<string | null>>();

let currentSessionPath: string | null = null;

/** 会话切换时调用：source_offset 只对当前文件有意义，换会话即整体失效 */
export function setToolResultSessionPath(path: string | null) {
  if (currentSessionPath === path) return;
  currentSessionPath = path;
  fullContentCache.clear();
  failedKeys.clear();
  pendingFetches.clear();
}

export function isLazyToolResult(part: LazyToolResultRef): boolean {
  return part.truncated_preview === true;
}

/**
 * 截断提示后缀：`true` 时在正文末尾追加本地化提示，否则空串。
 *
 * 后端不再把中文拼进 tool_result 正文（计划 10 的 D 类），只回 `truncated` 标志；
 * 开头的 `...\n\n` 是**排版**不是文案，与正文的拼接由前端完成。
 * 提示在渲染/复制时现取 `t()`，所以切语言会跟着变 —— 不会像 `reqMessages` 那样把
 * 取值烘进解析缓存（那是另一条管线：请求体预览的 32 KB 截断，见 `reqMessages.ts`）。
 */
export function toolResultTruncationSuffix(truncated?: boolean): string {
  return truncated ? `...\n\n${t("common.toolResult.truncatedSuffix")}` : "";
}

/** 渲染用：已缓存则返回全文，否则返回传入的（预览或全文）内容；被截断时追加本地化提示 */
export function displayToolResultContent(
  key: string,
  fallback: string,
  truncated?: boolean
): string {
  return `${fullContentCache.get(key) ?? fallback}${toolResultTruncationSuffix(truncated)}`;
}

export function toolResultFetchFailed(key: string): boolean {
  return failedKeys.has(key);
}

export function toolResultFetchPending(key: string): boolean {
  return pendingFetches.has(key);
}

/**
 * 确保拿到全文：已缓存直接返回；在途去重；失败返回 null（调用方回退预览）。
 * 非懒加载 part（小内容 / 白名单）直接返回 null —— 调用方本来就有全文，无需拉取。
 */
export function ensureToolResultFullContent(
  key: string,
  part: LazyToolResultRef
): Promise<string | null> {
  const cached = fullContentCache.get(key);
  if (cached !== undefined) return Promise.resolve(cached);
  if (!isLazyToolResult(part)) return Promise.resolve(null);
  if (!currentSessionPath || part.source_offset === undefined) return Promise.resolve(null);

  const pending = pendingFetches.get(key);
  if (pending) return pending;

  const promise = invokeApp<string>("get_tool_result_full_content", {
    filePath: currentSessionPath,
    sourceOffset: part.source_offset,
    toolUseId: part.tool_use_id ?? null,
  })
    .then((full) => {
      fullContentCache.set(key, full);
      failedKeys.delete(key);
      return full;
    })
    .catch((err) => {
      console.warn("[lazy-tool-result] 全文拉取失败，回退预览:", err);
      failedKeys.add(key);
      return null;
    })
    .finally(() => {
      pendingFetches.delete(key);
    });

  pendingFetches.set(key, promise);
  return promise;
}

/** 供 CopyButton 类同步接口使用：返回最终一定 resolve 为可复制文本的 Promise */
export async function copyableToolResultContent(
  key: string,
  part: LazyToolResultRef,
  fallback: string
): Promise<string> {
  const full = await ensureToolResultFullContent(key, part);
  // 复制文本与渲染文本保持一致：被截断时同样带提示（改造前标记就在正文里，复制也含它）
  return `${full ?? fallback}${toolResultTruncationSuffix(part.truncated)}`;
}

export type { ToolResultPart };
