import { invoke } from "@tauri-apps/api/core";
import { t } from "../i18n";

/** 后端结构化错误的线格式。批次 6 起 AppError 以此形状跨 IPC。 */
export interface CodedError {
  code: string;
  params?: Record<string, string>;
}

/**
 * 后端错误的线格式，三条错误通道共用一种：
 * `AppError::Coded` → `{ code, params }`，其余变体（含 `Business`）→ 裸字符串。
 *
 * 之所以是联合而不是只有对象：迁移期后端仍有大量 `AppError::business(...)` 站点
 * 发裸字符串（R1），两个形状必须同时成立，前端才能先就位、后端再逐步改造。
 */
export type AppErrorPayload = string | CodedError;

function isCodedError(value: unknown): value is CodedError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as CodedError).code === "string"
  );
}

/**
 * 取错误里的 code，取不到返回 `undefined`。两种形状都认：后端直接给的
 * `{ code, params }` 载荷（streaming / assistant 事件通道），以及 `wrapAppError`
 * 包装后挂在 `Error` 上的 `code`。
 *
 * **不是从渲染好的文案里找**：`renderAppError` 的输出是给人看的、随语言变，
 * 从它里面找 code 等于把「按后端文案子串判协议状态」换个名字装回去——
 * 那正是三处 `.vue` 与两处 composable 要拆掉的耦合。
 */
export function appErrorCode(err: unknown): string | undefined {
  return isCodedError(err) ? err.code : undefined;
}

/**
 * 把后端错误归一成 `Error`：`message` 是当前语言的文案，`code` 是后端的错误码（有则带上）。
 *
 * `code` 走**结构化字段**而不是塞进 `message`：协议状态判据必须独立于语言，
 * 否则译文一变（或换门语言）判据就静默失效。
 */
export function wrapAppError(err: unknown): Error & { code?: string } {
  const wrapped: Error & { code?: string } = new Error(renderAppError(err));
  if (isCodedError(err)) wrapped.code = err.code;
  return wrapped;
}

/**
 * 把后端错误渲染成用户可读文案。
 *
 * **同时认两种形状**：迁移期后端仍以字符串返回错误，批次 6 起改为
 * `{ code, params }`。两种都支持，前端才能先就位、后端再逐步改造，
 * 中途不存在「一半调用点拿到对象」的窗口。
 */
export function renderAppError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (isCodedError(err)) {
    const key = `errors.${err.code}`;
    const text = t(key, err.params ?? {});
    // 语言包缺条目时 t 返回 key 本身，此处回退为 code 便于定位
    return text === key ? err.code : text;
  }
  return String(err);
}

/**
 * `invoke` 的封装：错误统一经过 `renderAppError` 后再抛出。
 *
 * 调用方拿到的一律是 `Error`，其 `message` 已是当前语言的可读文案；后端以
 * `{ code, params }` 拒绝时 `code` 另挂在 `Error` 上（`appErrorCode` 取），
 * 供「按协议状态分支」的调用点使用。注意 Tauri 抛出的是**裸值**（迁移期是字符串），
 * 包成 `Error` 之后 `String(e)` 会渲染成 `"Error: 文案"`，
 * 故展示点必须走 `renderAppError(e)`（或读 `e.message`），不能直接 `String(e)`。
 */
export async function invokeApp<T>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  try {
    // 未传 args 时不补第二个实参：调用形状与直接调 invoke 完全一致，
    // 包装层不引入任何可观察差异。
    return args === undefined ? await invoke<T>(cmd) : await invoke<T>(cmd, args);
  } catch (err) {
    throw wrapAppError(err);
  }
}
