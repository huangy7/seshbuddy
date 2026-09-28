/**
 * 配置文件内容的纯取值/改写工具。
 *
 * 这些函数不依赖 Vue 响应式，也不碰组件状态，抽出来的唯一目的是**可单测**：
 * 嵌套路径的读写与空对象清理属于「边界一多就容易写错」的逻辑
 * （数组不是对象、中间层缺失、空字符串要删除而非写入空值），
 * 埋在 2000 行的组件里没有任何办法覆盖到。
 */

/** 把配置值转成输入框可显示的字符串。对象/数组一律显示为空，避免出现 "[object Object]"。 */
export function stringifyConfigValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

/** 按 "a.b.c" 点分路径取值。中间层缺失或不是对象时返回空串，不抛异常。 */
export function getNestedValue(content: Record<string, any>, key: string): string {
  const parts = key.split(".");
  let value: any = content;
  for (const part of parts) {
    if (value == null || typeof value !== "object") return "";
    value = value[part];
  }
  return stringifyConfigValue(value);
}

/**
 * 判断点分路径的**最后一段**是否作为自有键存在。
 *
 * 只看最后一段而不是整条链：中间层缺失时返回 false 即可，
 * 调用方关心的是「这个叶子键有没有被显式配置过」。
 * 数组不算对象 —— 数组下标不该被当成配置键处理。
 */
export function hasNestedKey(content: Record<string, any>, path: string[]): boolean {
  let value: any = content;
  for (let i = 0; i < path.length - 1; i += 1) {
    const part = path[i];
    if (!value || typeof value !== "object" || Array.isArray(value)) return false;
    value = value[part];
  }
  return Boolean(
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.prototype.hasOwnProperty.call(value, path[path.length - 1]),
  );
}

/**
 * 原地递归删除值为空对象的键。
 *
 * 清空某个输入框后，路径上会留下一串 `{}` 空壳；不清理的话，
 * 保存出去的配置里会带着 `"env": {}` 这类噪音，且再次读回时
 * `hasNestedKey` 会误判为「已配置过」。
 * 数组不参与清理（空数组是有意义的配置值）。
 */
export function pruneEmptyObjects(value: Record<string, any>): void {
  for (const key of Object.keys(value)) {
    const current = value[key];
    if (!current || typeof current !== "object" || Array.isArray(current)) continue;
    pruneEmptyObjects(current as Record<string, any>);
    if (Object.keys(current).length === 0) {
      delete value[key];
    }
  }
}
