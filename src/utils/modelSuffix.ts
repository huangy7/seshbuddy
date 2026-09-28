/**
 * `[1m]` 后缀工具：SeshBuddy 本地约定，模型 id 末尾的 `[1m]` 表示
 * 「向 Claude Code 声明支持 1M 上下文」，不是模型真实名称的一部分。
 * 与网关返回的模型 id 匹配前需先用 strip1m 剥离。
 */
export function has1m(model: string | undefined): boolean {
  return (model || "").trimEnd().toLowerCase().endsWith("[1m]");
}

export function strip1m(model: string | undefined): string {
  const trimmed = (model || "").trimEnd();
  if (!trimmed.toLowerCase().endsWith("[1m]")) return model || "";
  return trimmed.slice(0, -4).trimEnd();
}

export function set1m(model: string | undefined, enabled: boolean): string {
  const base = strip1m(model).trim();
  if (!base) return "";
  return enabled ? `${base}[1m]` : base;
}
