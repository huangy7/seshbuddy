import { ref, computed, watch, type Ref, type ComputedRef } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import { currentLocale, t } from "../i18n";
import type { RemoteModelPricing, PricingCatalogItem, PricingCatalogStatus } from "../types/pricing";

export const STORAGE_KEY_CATALOG = "seshbuddy-pricing-catalog-v2";
export const STORAGE_KEY_UPDATED_AT = "seshbuddy-pricing-updated-at-v2";

export const DEFAULT_MODEL_PRICING: RemoteModelPricing = {
  input_cost_per_token: 3 / 1_000_000,
  output_cost_per_token: 15 / 1_000_000,
  cache_creation_input_token_cost: 3.75 / 1_000_000,
  cache_read_input_token_cost: 0.3 / 1_000_000,
};

function createPricing(
  input: number,
  output: number,
  cacheWrite: number = input * 1.25,
  cacheRead: number = input * 0.1
): RemoteModelPricing {
  return {
    input_cost_per_token: input / 1_000_000,
    output_cost_per_token: output / 1_000_000,
    cache_creation_input_token_cost: cacheWrite / 1_000_000,
    cache_read_input_token_cost: cacheRead / 1_000_000,
  };
}

export const DEFAULT_CATALOG: Record<string, RemoteModelPricing> = {
  // Claude 4.6
  "anthropic/claude-opus-4-6": createPricing(5, 25, 6.25, 0.5),
  "claude-opus-4-6": createPricing(5, 25, 6.25, 0.5),
  "anthropic/claude-sonnet-4-6": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4-6": createPricing(3, 15, 3.75, 0.3),

  // Claude 4.5
  "anthropic/claude-opus-4-5": createPricing(5, 25, 6.25, 0.5),
  "claude-opus-4-5": createPricing(5, 25, 6.25, 0.5),
  "anthropic/claude-sonnet-4-5": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4-5": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-haiku-4-5": createPricing(1, 5, 1.25, 0.1),
  "claude-haiku-4-5": createPricing(1, 5, 1.25, 0.1),

  // Claude 4 (Legacy / Date suffixes)
  "anthropic/claude-sonnet-4": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-sonnet-4-20250514": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4-20250514": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-opus-4-20250514": createPricing(15, 75, 18.75, 1.5),
  "claude-opus-4-20250514": createPricing(15, 75, 18.75, 1.5),
  "anthropic/claude-haiku-4-20250414": createPricing(0.8, 4, 1, 0.08),
  "claude-haiku-4-20250414": createPricing(0.8, 4, 1, 0.08),

  // Claude 3.5 & 3
  "anthropic/claude-3-5-sonnet": createPricing(3, 15, 3.75, 0.3),
  "claude-3-5-sonnet": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-3-5-haiku": createPricing(0.8, 4, 1, 0.08),
  "claude-3-5-haiku": createPricing(0.8, 4, 1, 0.08),
  "anthropic/claude-3-opus": createPricing(15, 75, 18.75, 1.5),
  "claude-3-opus": createPricing(15, 75, 18.75, 1.5),
  "anthropic/claude-3-haiku": createPricing(0.25, 1.25, 0.3, 0.03),
  "claude-3-haiku": createPricing(0.25, 1.25, 0.3, 0.03),

  // Google Gemini
  "google/gemini-2.5-pro": createPricing(1.25, 5, 1.25, 0.3125),
  "gemini-2.5-pro": createPricing(1.25, 5, 1.25, 0.3125),
  "google/gemini-2.5-flash": createPricing(0.15, 0.6, 0.15, 0.0375),
  "gemini-2.5-flash": createPricing(0.15, 0.6, 0.15, 0.0375),
  "google/gemini-2.0-flash": createPricing(0.1, 0.4, 0.1, 0.025),
  "gemini-2.0-flash": createPricing(0.1, 0.4, 0.1, 0.025),

  // OpenAI
  "openai/gpt-4o": createPricing(2.5, 10, 2.5, 1.25),
  "gpt-4o": createPricing(2.5, 10, 2.5, 1.25),
  "openai/gpt-4o-mini": createPricing(0.15, 0.6, 0.15, 0.075),
  "gpt-4o-mini": createPricing(0.15, 0.6, 0.15, 0.075),

  // DeepSeek
  "deepseek/deepseek-chat": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek-chat": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek/deepseek-v3": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek-v3": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek/deepseek-reasoner": createPricing(0.55, 2.19, 0.55, 0.14),
  "deepseek-reasoner": createPricing(0.55, 2.19, 0.55, 0.14),

  // Moonshot / Kimi
  "moonshotai/kimi-k3": createPricing(3, 15, 3.75, 0.3),
  "kimi-k3": createPricing(3, 15, 3.75, 0.3),
  "moonshotai/kimi-k2.7-code": createPricing(0.95, 4, 1, 0.19),
  "kimi-k2.7-code": createPricing(0.95, 4, 1, 0.19),
  "moonshotai/kimi-k2.5": createPricing(0.6, 2.5, 0.6, 0.15),
  "kimi-k2.5": createPricing(0.6, 2.5, 0.6, 0.15),
  "moonshotai/kimi-k2-thinking": createPricing(0.6, 2.5, 0.6, 0.15),
  "kimi-k2-thinking": createPricing(0.6, 2.5, 0.6, 0.15),
  "moonshotai/moonshot-v1-8k": createPricing(1.68, 1.68, 1.68, 0.84),
  "moonshot-v1-8k": createPricing(1.68, 1.68, 1.68, 0.84),
  "moonshotai/moonshot-v1-32k": createPricing(3.36, 3.36, 3.36, 1.68),
  "moonshot-v1-32k": createPricing(3.36, 3.36, 3.36, 1.68),
  "moonshotai/moonshot-v1-128k": createPricing(8.4, 8.4, 8.4, 4.2),
  "moonshot-v1-128k": createPricing(8.4, 8.4, 8.4, 4.2),

  // Alibaba / 通义千问
  "alibaba/qwen3.7-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "qwen3.7-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "alibaba/qwen-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "qwen-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "alibaba/qwen-plus": createPricing(0.4, 1.2, 0.4, 0.1),
  "qwen-plus": createPricing(0.4, 1.2, 0.4, 0.1),
  "alibaba/qwen-turbo": createPricing(0.04, 0.08, 0.04, 0.01),
  "qwen-turbo": createPricing(0.04, 0.08, 0.04, 0.01),

  // Zhipu / 智谱 GLM
  "zhipuai/glm-5": createPricing(1, 3.2, 1, 0.2),
  "glm-5": createPricing(1, 3.2, 1, 0.2),
  "zhipuai/glm-4.7": createPricing(0.8, 2.8, 0.8, 0.16),
  "glm-4.7": createPricing(0.8, 2.8, 0.8, 0.16),
  "zhipuai/glm-4.6": createPricing(0.6, 2.2, 0.6, 0.11),
  "glm-4.6": createPricing(0.6, 2.2, 0.6, 0.11),
  "zhipuai/glm-4-flash": createPricing(0.01, 0.01, 0.01, 0.005),
  "glm-4-flash": createPricing(0.01, 0.01, 0.01, 0.005),

  // MiniMax
  "minimax/minimax-m2.7": createPricing(0.3, 1.2, 0.375, 0.06),
  "minimax-m2.7": createPricing(0.3, 1.2, 0.375, 0.06),
  "minimax/minimax-m2.5": createPricing(0.2, 0.8, 0.25, 0.04),
  "minimax-m2.5": createPricing(0.2, 0.8, 0.25, 0.04),

  // StepFun / 阶跃星辰
  "stepfun/step-3.5-flash": createPricing(0.1, 0.3, 0.1, 0.02),
  "step-3.5-flash": createPricing(0.1, 0.3, 0.1, 0.02),
};

function loadPersistedCatalog(): Record<string, RemoteModelPricing> | null {
  try {
    const saved = localStorage.getItem(STORAGE_KEY_CATALOG);
    if (saved) {
      const parsed = JSON.parse(saved);
      if (parsed && typeof parsed === "object" && Object.keys(parsed).length > 0) {
        return { ...DEFAULT_CATALOG, ...parsed };
      }
    }
  } catch {
    // Ignore localStorage errors
  }
  return null;
}

function loadPersistedUpdatedAt(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY_UPDATED_AT);
  } catch {
    return null;
  }
}

// Singleton state
const catalog = ref<Record<string, RemoteModelPricing>>(loadPersistedCatalog() || { ...DEFAULT_CATALOG });
const updatedAt = ref<string | null>(loadPersistedUpdatedAt());
const isRefreshing = ref<boolean>(false);
const refreshError = ref<string>("");

export function normalizeModelKey(model: string): string {
  if (!model) return "";
  return model
    .trim()
    .toLowerCase()
    .replace(/^models\//, "");
}

export function stripVersionAndVariant(model: string): string[] {
  const normalized = normalizeModelKey(model);
  if (!normalized) return [];

  const candidates = new Set<string>();

  // Extract provider & pure model if / exists
  let pureModel = normalized;
  let providerPrefix = "";
  if (normalized.includes("/")) {
    const slashIdx = normalized.indexOf("/");
    providerPrefix = normalized.slice(0, slashIdx + 1);
    pureModel = normalized.slice(slashIdx + 1);
    candidates.add(pureModel);
  }

  // Variations to transform
  const bases = [pureModel];
  if (providerPrefix) {
    bases.push(normalized);
  }

  for (const base of bases) {
    // 1. Strip date: -20250514, -2024-08-06, -0514, etc.
    const withoutDate = base
      .replace(/[-_.]\d{4}-\d{2}-\d{2}$/, "")
      .replace(/[-_.]\d{8}$/, "")
      .replace(/[-_.]\d{4}$/, "");
    if (withoutDate && withoutDate !== base) {
      candidates.add(withoutDate);
      if (withoutDate.includes("/")) {
        candidates.add(withoutDate.slice(withoutDate.indexOf("/") + 1));
      }
    }

    // 2. Strip variant suffixes: -latest, -preview, -fast, -mini, -turbo, -pro, -lite, -build, etc.
    const withoutVariant = base
      .replace(/-(?:latest|preview|fast|turbo|mini|pro|lite|build|chat|reasoner)(?:-\d+)?$/g, "");
    if (withoutVariant && withoutVariant !== base) {
      candidates.add(withoutVariant);
      if (withoutVariant.includes("/")) {
        candidates.add(withoutVariant.slice(withoutVariant.indexOf("/") + 1));
      }
    }

    // 3. Combined date + variant strip
    const withoutBoth = withoutDate
      .replace(/-(?:latest|preview|fast|turbo|mini|pro|lite|build|chat|reasoner)(?:-\d+)?$/g, "");
    if (withoutBoth && withoutBoth !== base) {
      candidates.add(withoutBoth);
      if (withoutBoth.includes("/")) {
        candidates.add(withoutBoth.slice(withoutBoth.indexOf("/") + 1));
      }
    }

    // 4. Dot vs Dash version replacement: claude-3.5-sonnet <-> claude-3-5-sonnet
    if (/\d+\.\d+/.test(base)) {
      const dashed = base.replace(/(\d+)\.(\d+)/g, "$1-$2");
      candidates.add(dashed);
      if (dashed.includes("/")) {
        candidates.add(dashed.slice(dashed.indexOf("/") + 1));
      }
    }
    if (/\d+-\d+/.test(base)) {
      const dotted = base.replace(/(\d+)-(\d+)/g, "$1.$2");
      candidates.add(dotted);
      if (dotted.includes("/")) {
        candidates.add(dotted.slice(dotted.indexOf("/") + 1));
      }
    }
  }

  candidates.delete(normalized);
  return Array.from(candidates);
}

const pricingCache = new Map<string, RemoteModelPricing>();
watch(catalog, () => pricingCache.clear(), { deep: true });

export function getModelPricing(model: string): RemoteModelPricing {
  if (!model) return DEFAULT_MODEL_PRICING;
  const key = normalizeModelKey(model);
  if (!key) return DEFAULT_MODEL_PRICING;

  const cached = pricingCache.get(key);
  if (cached) return cached;

  let result: RemoteModelPricing | undefined;

  // 1. Exact match in catalog
  if (catalog.value[key]) {
    result = catalog.value[key];
  }

  // 2. Candidate match
  if (!result) {
    const candidates = stripVersionAndVariant(key);
    for (const candidate of candidates) {
      if (catalog.value[candidate]) {
        result = catalog.value[candidate];
        break;
      }
    }
  }

  // 3. Keyword heuristic matching (O(1))
  if (!result) {
    if (key.includes("opus")) result = catalog.value["claude-opus-4-6"] || DEFAULT_CATALOG["claude-opus-4-6"];
    else if (key.includes("haiku")) result = catalog.value["claude-3-5-haiku"] || DEFAULT_CATALOG["claude-3-5-haiku"];
    else if (key.includes("flash")) result = catalog.value["gemini-2.5-flash"] || DEFAULT_CATALOG["gemini-2.5-flash"];
    else if (key.includes("sonnet")) result = catalog.value["claude-sonnet-4-6"] || DEFAULT_CATALOG["claude-sonnet-4-6"];
    else if (key.includes("gpt-4o-mini")) result = catalog.value["gpt-4o-mini"] || DEFAULT_CATALOG["gpt-4o-mini"];
    else if (key.includes("gpt-4") || key.includes("gpt-4o")) result = catalog.value["gpt-4o"] || DEFAULT_CATALOG["gpt-4o"];
    else if (key.includes("deepseek")) result = catalog.value["deepseek-chat"] || DEFAULT_CATALOG["deepseek-chat"];
    else if (key.includes("kimi") || key.includes("moonshot")) result = catalog.value["kimi-k3"] || DEFAULT_CATALOG["kimi-k3"];
    else if (key.includes("qwen")) result = catalog.value["qwen-max"] || DEFAULT_CATALOG["qwen-max"];
    else if (key.includes("glm")) result = catalog.value["glm-5"] || DEFAULT_CATALOG["glm-5"];
    else if (key.includes("minimax") || key.includes("abab")) result = catalog.value["minimax-m2.7"] || DEFAULT_CATALOG["minimax-m2.7"];
    else if (key.includes("step")) result = catalog.value["step-3.5-flash"] || DEFAULT_CATALOG["step-3.5-flash"];
    else if (key.includes("grok")) result = catalog.value["grok-4.5"] || DEFAULT_CATALOG["grok-4.5"];
  }

  const finalPricing = result || DEFAULT_MODEL_PRICING;
  pricingCache.set(key, finalPricing);
  return finalPricing;
}

export interface PricingCatalogResponse {
  catalog: Record<string, RemoteModelPricing>;
  updated_at: string | null;
  model_count: number;
}

export async function fetchPricingCatalog(force = false): Promise<void> {
  if (isRefreshing.value) return;
  if (!force && updatedAt.value && Object.keys(catalog.value).length > 0) return;

  isRefreshing.value = true;
  refreshError.value = "";

  try {
    // 1. Try native Rust backend first (fast reqwest, system proxy, gzip decompression, SQLite caching)
    try {
      const res = await invokeApp<PricingCatalogResponse>("refresh_pricing_catalog");
      if (res && res.catalog && Object.keys(res.catalog).length > 0) {
        const nextCatalog = { ...DEFAULT_CATALOG, ...res.catalog };
        const now = res.updated_at || new Date().toISOString();
        pricingCache.clear();
        catalog.value = nextCatalog;
        updatedAt.value = now;
        try {
          localStorage.setItem(STORAGE_KEY_CATALOG, JSON.stringify(nextCatalog));
          localStorage.setItem(STORAGE_KEY_UPDATED_AT, now);
        } catch {
          // Ignore localStorage errors
        }
        return;
      }
    } catch {
      // Fallback to web fetch below
    }

    // 2. Web / Vitest fallback
    const res = await fetch("https://models.dev/api.json");
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${res.statusText}`);
    }
    const data = await res.json();
    const nextCatalog: Record<string, RemoteModelPricing> = { ...DEFAULT_CATALOG };

    if (data && typeof data === "object") {
      if (Array.isArray(data)) {
        for (const item of data) {
          if (item && typeof item === "object") {
            const provider = String(item.provider || item.providerId || "custom").toLowerCase();
            const model = String(item.id || item.model || item.name || "").toLowerCase();
            if (model) {
              const cost = item.cost || item.pricing || {};
              const pricing: RemoteModelPricing = {
                input_cost_per_token: cost.input != null ? Number(cost.input) / 1_000_000 : undefined,
                output_cost_per_token: cost.output != null ? Number(cost.output) / 1_000_000 : undefined,
                cache_read_input_token_cost: cost.cache_read != null ? Number(cost.cache_read) / 1_000_000 : undefined,
                cache_creation_input_token_cost:
                  cost.cache_write != null
                    ? Number(cost.cache_write) / 1_000_000
                    : cost.cache_creation != null
                    ? Number(cost.cache_creation) / 1_000_000
                    : undefined,
              };
              nextCatalog[`${provider}/${model}`] = pricing;
              nextCatalog[model] = pricing;
            }
          }
        }
      } else {
        const PRIMARY_KEYS = [
          "anthropic",
          "openai",
          "google",
          "deepseek",
          "moonshotai",
          "moonshotai-cn",
          "kimi-for-coding",
          "zhipuai",
          "zai",
          "alibaba",
          "alibaba-cn",
          "minimax",
          "minimax-cn",
          "stepfun",
          "stepfun-ai",
          "xai",
          "meta",
          "mistral",
          "baichuan",
          "volcengine",
        ];
        for (const [providerKey, providerObj] of Object.entries(data as Record<string, any>)) {
          if (!providerObj || typeof providerObj !== "object") continue;
          const pKey = providerKey.toLowerCase();
          if (!PRIMARY_KEYS.some((k) => pKey === k || pKey.startsWith(k))) continue;
          const models = providerObj.models || (providerObj.cost ? { [providerKey]: providerObj } : {});
          for (const [modelKey, modelObj] of Object.entries(models as Record<string, any>)) {
            if (!modelObj || typeof modelObj !== "object") continue;
            const mKey = (modelObj.id || modelKey).toLowerCase();
            const cost = modelObj.cost || modelObj.pricing || {};
            const pricing: RemoteModelPricing = {
              input_cost_per_token: cost.input != null ? Number(cost.input) / 1_000_000 : undefined,
              output_cost_per_token: cost.output != null ? Number(cost.output) / 1_000_000 : undefined,
              cache_read_input_token_cost: cost.cache_read != null ? Number(cost.cache_read) / 1_000_000 : undefined,
              cache_creation_input_token_cost:
                cost.cache_write != null
                  ? Number(cost.cache_write) / 1_000_000
                  : cost.cache_creation != null
                  ? Number(cost.cache_creation) / 1_000_000
                  : undefined,
            };
            nextCatalog[`${pKey}/${mKey}`] = pricing;
            nextCatalog[mKey] = pricing;
          }
        }
      }
    }

    const now = new Date().toISOString();
    pricingCache.clear();
    catalog.value = nextCatalog;
    updatedAt.value = now;

    try {
      localStorage.setItem(STORAGE_KEY_CATALOG, JSON.stringify(nextCatalog));
      localStorage.setItem(STORAGE_KEY_UPDATED_AT, now);
    } catch {
      // Ignore localStorage errors
    }
  } catch (err: unknown) {
    refreshError.value = renderAppError(err);
  } finally {
    isRefreshing.value = false;
  }
}

/**
 * 厂商 id 词表：18 个已知厂商的稳定标识，与语言包 `dialogs.pricing.provider.<slug>` 一一对应。
 *
 * **身份与显示在这里分开。** id 是数据：它进 `${id}::${model}` 去重键、进下拉项的 value、
 * 进过滤比较；显示名一律经 providerDisplayName 现取。反过来做（拿显示名当身份）时，
 * 翻译任一处厂商名就会让同一厂商裂成两个下拉项、排序错乱、过滤静默变空——三种失败都不报错。
 */
export const PROVIDER_IDS = [
  "anthropic",
  "openai",
  "google",
  "deepseek",
  "moonshot-kimi",
  "zhipu",
  "alibaba",
  "minimax",
  "stepfun",
  "baichuan",
  "bytedance",
  "tencent",
  "baidu",
  "siliconflow",
  "meta",
  "mistral",
  "groq",
  "xai",
] as const;

const KNOWN_PROVIDER_IDS = new Set<string>(PROVIDER_IDS);

/** 18 个已知 slug，或未知厂商的规范化回退值（见 resolveProviderId）。 */
export type ProviderId = string;

/**
 * 「当前会话」哨兵项的 id：`usedItems` 兜底链里认不出厂商的模型归到它名下。
 * 刻意不落在 PROVIDER_IDS 里，故不可能与真实厂商碰撞；显示名另有语言包键
 * `dialogs.pricing.providerCurrentSession`（见 providerDisplayName）。
 */
export const CURRENT_SESSION_PROVIDER_ID = "current-session";

/**
 * 厂商 id 的**唯一入口**：三处调用点（带 `provider/` 前缀的键、裸模型名、usedItems 兜底链）
 * 都调它，故同一厂商不可能在两条路径上落到不同 id。
 *
 * 输入既可以是 `provider/model` 的前缀，也可以是裸模型名——判据是关键词而不是形状，
 * 故两种形状共用一套分支。分支里既有厂商名别名（`zhipuai` / `volcengine` / `zai`…），
 * 也有模型名前缀（`glm` / `doubao` / `llama` / `gpt` / `o1`…）：裸模型名没有厂商前缀可依，
 * 只能按模型家族判。
 *
 * 认不出时**沿用原有回退形态**（首字母大写，逐字保留原大小写）：未知厂商的 id 就是那个回退串。
 * 不要把未知厂商并成一个共用桶——`availableProviders` 只滤掉大小写敏感的 `"Other"`，
 * 并桶会让所有未知厂商在下拉项里合并成一条。
 */
export function resolveProviderId(raw: string): ProviderId {
  const p = raw.toLowerCase();
  if (p.includes("anthropic") || p.includes("claude")) return "anthropic";
  // `gpt` / `o1` / `o3` 是模型名前缀而非厂商名，裸模型名只有这一条路认得出它们。
  if (
    p.includes("openai") ||
    p.includes("chatgpt") ||
    p.includes("codex") ||
    p.startsWith("gpt") ||
    p.startsWith("o1") ||
    p.startsWith("o3")
  ) {
    return "openai";
  }
  if (p.includes("google") || p.includes("gemini") || p.includes("vertex")) return "google";
  if (p.includes("deepseek")) return "deepseek";
  if (p.includes("moonshot") || p.includes("kimi")) return "moonshot-kimi";
  if (p.includes("zhipu") || p.includes("zai") || p.includes("glm")) return "zhipu";
  if (p.includes("alibaba") || p.includes("qwen") || p.includes("dashscope") || p.includes("aliyun")) return "alibaba";
  if (p.includes("minimax") || p.includes("abab")) return "minimax";
  if (p.includes("stepfun") || p.includes("step")) return "stepfun";
  if (p.includes("baichuan")) return "baichuan";
  if (p.includes("volcengine") || p.includes("bytedance") || p.includes("doubao")) return "bytedance";
  if (p.includes("tencent") || p.includes("hunyuan")) return "tencent";
  if (p.includes("baidu") || p.includes("qianfan") || p.includes("ernie")) return "baidu";
  if (p.includes("siliconflow") || p.includes("silicon")) return "siliconflow";
  if (p.includes("meta") || p.includes("llama")) return "meta";
  if (p.includes("mistral")) return "mistral";
  if (p.includes("groq")) return "groq";
  if (p.includes("xai") || p.includes("grok")) return "xai";
  return raw.charAt(0).toUpperCase() + raw.slice(1);
}

/** 这个 id 是不是词表里的已知厂商（回退串与哨兵 id 都返回 false）。 */
export function isKnownProviderId(id: ProviderId): boolean {
  return KNOWN_PROVIDER_IDS.has(id);
}

/**
 * 显示出口：id → 界面可见的厂商名。**唯一的 t() 出口**，组件与数据源都走它。
 *
 * 未知 id **原样返回**（它就是品牌名），不能直接丢给 t()——vue-i18n 缺键时会把 key path
 * 渲染到界面上并告警。哨兵 id 的显示名另有键，它不是厂商。
 */
export function providerDisplayName(id: ProviderId): string {
  if (id === CURRENT_SESSION_PROVIDER_ID) return t("dialogs.pricing.providerCurrentSession");
  return isKnownProviderId(id) ? t(`dialogs.pricing.provider.${id}`) : id;
}

const catalogItems = computed<PricingCatalogItem[]>(() => {
  const items: PricingCatalogItem[] = [];
  const seen = new Set<string>();

  // Pass 1: Keys with provider/model
  for (const [key, pricing] of Object.entries(catalog.value)) {
    if (key.includes("/")) {
      const slashIdx = key.indexOf("/");
      const providerRaw = key.slice(0, slashIdx);
      const model = key.slice(slashIdx + 1);
      const provider = resolveProviderId(providerRaw);
      const uniqueKey = `${provider}::${model.toLowerCase()}`;
      if (seen.has(uniqueKey)) continue;
      seen.add(uniqueKey);

      const inputPer1M = Number(((pricing.input_cost_per_token || 0) * 1_000_000).toFixed(4));
      const outputPer1M = Number(((pricing.output_cost_per_token || 0) * 1_000_000).toFixed(4));
      const cacheWritePer1M = Number(((pricing.cache_creation_input_token_cost || 0) * 1_000_000).toFixed(4));
      const cacheReadPer1M = Number(((pricing.cache_read_input_token_cost || 0) * 1_000_000).toFixed(4));

      items.push({
        key,
        model,
        provider,
        inputPer1M,
        outputPer1M,
        cacheWritePer1M,
        cacheReadPer1M,
      });
    }
  }

  // Pass 2: Keys without slash that are not in seen
  for (const [key, pricing] of Object.entries(catalog.value)) {
    if (!key.includes("/")) {
      // 裸模型名没有厂商前缀：同一个 resolver 认得出就取那个 id，认不出时沿用本 pass
      // 原有的 "Other" 哨兵——下面按 DEFAULT_CATALOG 整条丢弃，与拆开之前逐条一致。
      const resolved = resolveProviderId(key);
      const inferredProvider = isKnownProviderId(resolved) ? resolved : "Other";

      const uniqueKey = `${inferredProvider}::${key.toLowerCase()}`;
      if (seen.has(uniqueKey)) continue;
      seen.add(uniqueKey);

      if (inferredProvider === "Other" && !DEFAULT_CATALOG[key]) continue;

      const inputPer1M = Number(((pricing.input_cost_per_token || 0) * 1_000_000).toFixed(4));
      const outputPer1M = Number(((pricing.output_cost_per_token || 0) * 1_000_000).toFixed(4));
      const cacheWritePer1M = Number(((pricing.cache_creation_input_token_cost || 0) * 1_000_000).toFixed(4));
      const cacheReadPer1M = Number(((pricing.cache_read_input_token_cost || 0) * 1_000_000).toFixed(4));

      items.push({
        key,
        model: key,
        provider: inferredProvider,
        inputPer1M,
        outputPer1M,
        cacheWritePer1M,
        cacheReadPer1M,
      });
    }
  }

  // 排序用的是**界面可见的厂商名**（身份是 id，不再是可见文本）：不传 locale 时
  // localeCompare 走宿主默认，与界面语言不一致时同一份清单的先后会变。
  // 显示名按 id 缓存一次，别在比较器里反复求值。
  const labels = new Map<string, string>();
  const labelOf = (id: string): string => {
    let name = labels.get(id);
    if (name === undefined) {
      name = providerDisplayName(id);
      labels.set(id, name);
    }
    return name;
  };

  return items
    .map((item) => ({ item, label: labelOf(item.provider) }))
    .sort(
      (a, b) =>
        a.label.localeCompare(b.label, currentLocale.value) ||
        a.item.model.localeCompare(b.item.model, currentLocale.value)
    )
    .map(({ item }) => item);
});

const status = computed<PricingCatalogStatus>(() => ({
  model_count: catalogItems.value.length,
  updated_at: updatedAt.value,
}));

export function resetPricingCatalogStore(): void {
  pricingCache.clear();
  catalog.value = loadPersistedCatalog() || { ...DEFAULT_CATALOG };
  updatedAt.value = loadPersistedUpdatedAt();
  isRefreshing.value = false;
  refreshError.value = "";
}

export function usePricingCatalog() {
  return {
    catalog: catalog as Ref<Record<string, RemoteModelPricing>>,
    catalogItems: catalogItems as ComputedRef<PricingCatalogItem[]>,
    status: status as ComputedRef<PricingCatalogStatus>,
    updatedAt: updatedAt as Ref<string | null>,
    isRefreshing: isRefreshing as Ref<boolean>,
    refreshError: refreshError as Ref<string>,
    fetchPricingCatalog,
    getModelPricing,
  };
}
