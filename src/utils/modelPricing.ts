import type { ModelEntry } from "./modelDisplayName";
import { t } from "../i18n";

/**
 * 网关 billing_rules 计费公式解析。
 * 公式形如 tier("base", p * 5 + c * 25 + cr * 0.5 + cc * 6.25 + cc1h * 10)，
 * 系数单位是「$ / 百万 token」。变量含义：
 *   p 输入（缓存未命中）/ c 输出 / cr 缓存读 / cc 缓存写(5m) / cc1h 缓存写(1h)
 * 多 tier 分层时只取第一档（base）系数，分层条件忽略。
 */
export interface BillingCoefficients {
  p?: number;
  c?: number;
  cr?: number;
  cc?: number;
  cc1h?: number;
}

const BILLING_VARS = ["p", "c", "cr", "cc", "cc1h"] as const;

export function parseBillingRules(rules: string | null | undefined): BillingCoefficients {
  const coeffs: BillingCoefficients = {};
  if (!rules) return coeffs;
  for (const v of BILLING_VARS) {
    // \b 词边界避免 c 误匹配 cr/cc/cc1h 内部
    const m = rules.match(new RegExp(`\\b${v}\\s*\\*\\s*([0-9]+(?:\\.[0-9]+)?)`));
    if (m) coeffs[v] = parseFloat(m[1]);
  }
  return coeffs;
}

/** 精简价格："$in/$out"（$ / 百万 token）；无输入价则返回 null */
export function formatModelPrice(coeffs: BillingCoefficients): string | null {
  if (coeffs.p == null) return null;
  const out = coeffs.c != null ? `$${coeffs.c}` : "—";
  return `$${coeffs.p}/${out}`;
}

/**
 * 下拉项 hover 提示：有价格时「id · $in/$out per M」，否则仅 id。
 *
 * 单位词 `per M` 是英文界面文案（不是格式名或标识符），闸门规则 1 只认 CJK 因而看不见它；
 * 走语言包后 ja/de 用户才拿得到可读的单位，en/zh 取值与迁移前逐字相同。
 */
export function formatModelTooltip(id: string, priceById?: Map<string, string>): string {
  const price = priceById?.get(id);
  return price ? t("common.modelPricing.tooltip", { id, price }) : id;
}

/** id → 精简价格字符串映射；无法解析出价格的条目不收录 */
export function buildPriceById(entries: ModelEntry[]): Map<string, string> {
  const map = new Map<string, string>();
  for (const entry of entries) {
    const price = formatModelPrice(parseBillingRules(entry.billing_rules));
    if (price) map.set(entry.id, price);
  }
  return map;
}
