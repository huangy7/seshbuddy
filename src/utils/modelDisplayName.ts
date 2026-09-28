/** 网关 /v1/models 返回的模型条目（display / billing_rules 可能缺失） */
export interface ModelEntry {
  id: string;
  display?: string | null;
  billing_rules?: string | null;
}

/** id → display 映射；无 display（或纯空白）的条目不收录 */
export function buildDisplayById(entries: ModelEntry[]): Map<string, string> {
  const map = new Map<string, string>();
  for (const entry of entries) {
    const display = entry.display?.trim();
    if (display) map.set(entry.id, display);
  }
  return map;
}

type ModelRole = "haiku" | "sonnet" | "opus";

const MODEL_ROLES = ["haiku", "sonnet", "opus"] as const;

/**
 * 认定配置里读出来的名字是「手填」还是「自动」。
 * 等于当前模型 display 的，说明是旧版本自动写进去的 → 保持未手填，让它继续跟随模型；
 * 其他非空名字 → 认定手填，之后切模型不再覆盖。已经手填过的角色不会被改回去。
 */
export function adoptLoadedNames(
  currentNames: Record<ModelRole, string>,
  autoNames: Record<ModelRole, string>,
  typed: Record<ModelRole, boolean>,
): Record<ModelRole, boolean> {
  const next = { ...typed };
  for (const role of MODEL_ROLES) {
    if (next[role]) continue;
    const current = currentNames[role].trim();
    if (current !== "" && current !== autoNames[role].trim()) next[role] = true;
  }
  return next;
}

/**
 * 把未手填的角色行同步为网关 display。
 * 返回空串表示该行应当清空（新模型没有 display），不是「跳过」。
 */
export function fillAutoNames(
  autoNames: Record<ModelRole, string>,
  typed: Record<ModelRole, boolean>,
): Partial<Record<ModelRole, string>> {
  const result: Partial<Record<ModelRole, string>> = {};
  for (const role of MODEL_ROLES) {
    if (!typed[role]) result[role] = autoNames[role];
  }
  return result;
}

/**
 * 解析 localStorage 里的模型列表缓存。
 * 兼容旧格式 { model_ids: string[] }（升级为无 display 的条目）与新格式 { models: ModelEntry[] }。
 */
export function parseCachedModelEntries(raw: string | null): ModelEntry[] {
  if (!raw) return [];
  try {
    const cached = JSON.parse(raw);
    if (Array.isArray(cached?.models)) {
      return cached.models
        .filter((m: unknown) => m && typeof (m as ModelEntry).id === "string")
        .map((m: ModelEntry) => ({
          id: m.id,
          display: m.display ?? undefined,
          billing_rules: m.billing_rules ?? undefined,
        }));
    }
    if (Array.isArray(cached?.model_ids)) {
      return cached.model_ids
        .filter((id: unknown) => typeof id === "string")
        .map((id: string) => ({ id }));
    }
  } catch {
    // fall through
  }
  return [];
}
