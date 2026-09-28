import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  usePricingCatalog,
  normalizeModelKey,
  stripVersionAndVariant,
  getModelPricing,
  resetPricingCatalogStore,
  resolveProviderId,
  isKnownProviderId,
  providerDisplayName,
  CURRENT_SESSION_PROVIDER_ID,
  PROVIDER_IDS,
  STORAGE_KEY_CATALOG,
  STORAGE_KEY_UPDATED_AT,
  DEFAULT_CATALOG,
  DEFAULT_MODEL_PRICING,
} from "./usePricingCatalog";
import { t } from "../i18n";
import enDialogs from "../locales/en/dialogs.json";
import zhDialogs from "../locales/zh/dialogs.json";
import jaDialogs from "../locales/ja/dialogs.json";
import deDialogs from "../locales/de/dialogs.json";

describe("usePricingCatalog 模型价格目录与匹配引擎", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, val: string) => {
        store[key] = val;
      },
      removeItem: (key: string) => {
        delete store[key];
      },
      clear: () => {
        store = {};
      },
    });
    resetPricingCatalogStore();
    vi.restoreAllMocks();
  });

  afterEach(() => {
    store = {};
    resetPricingCatalogStore();
    vi.restoreAllMocks();
  });

  describe("normalizeModelKey", () => {
    it("去除首尾空格、转换为小写，并移除 models/ 前缀", () => {
      expect(normalizeModelKey("  Claude-3-5-Sonnet-20241022  ")).toBe("claude-3-5-sonnet-20241022");
      expect(normalizeModelKey("models/gemini-2.5-flash")).toBe("gemini-2.5-flash");
      expect(normalizeModelKey("OPENAI/GPT-4O")).toBe("openai/gpt-4o");
    });

    it("空字符串或 falsy 输入返回空字符串", () => {
      expect(normalizeModelKey("")).toBe("");
      expect(normalizeModelKey("   ")).toBe("");
    });
  });

  describe("stripVersionAndVariant", () => {
    it("剔除 8 位及连字符日期后缀", () => {
      const candidates1 = stripVersionAndVariant("claude-sonnet-4-20250514");
      expect(candidates1).toContain("claude-sonnet-4");

      const candidates2 = stripVersionAndVariant("gpt-4o-2024-08-06");
      expect(candidates2).toContain("gpt-4o");
    });

    it("剔除变体后缀 (-latest, -preview, -fast, -mini, -turbo 等)", () => {
      const candidates1 = stripVersionAndVariant("gemini-2.5-flash-latest");
      expect(candidates1).toContain("gemini-2.5-flash");

      const candidates2 = stripVersionAndVariant("claude-3-5-sonnet-preview");
      expect(candidates2).toContain("claude-3-5-sonnet");
    });

    it("剥离厂商前缀生成别名候选", () => {
      const candidates = stripVersionAndVariant("anthropic/claude-3-5-sonnet-20241022");
      expect(candidates).toContain("claude-3-5-sonnet-20241022");
      expect(candidates).toContain("anthropic/claude-3-5-sonnet");
      expect(candidates).toContain("claude-3-5-sonnet");
    });

    it("支持点版本号与短横线版本号转换", () => {
      const candidates = stripVersionAndVariant("claude-3.5-sonnet");
      expect(candidates).toContain("claude-3-5-sonnet");
    });
  });

  describe("getModelPricing 智能价格匹配", () => {
    it("精确匹配内置目录中的模型", () => {
      const pricing = getModelPricing("claude-3-5-sonnet");
      expect(pricing.input_cost_per_token).toBeCloseTo(3 / 1_000_000);
      expect(pricing.output_cost_per_token).toBeCloseTo(15 / 1_000_000);
      expect(pricing.cache_creation_input_token_cost).toBeCloseTo(3.75 / 1_000_000);
      expect(pricing.cache_read_input_token_cost).toBeCloseTo(0.3 / 1_000_000);
    });

    it("通过剥离日期后缀匹配模型定价", () => {
      const pricing = getModelPricing("claude-sonnet-4-20250514");
      expect(pricing.input_cost_per_token).toBeCloseTo(3 / 1_000_000);
      expect(pricing.output_cost_per_token).toBeCloseTo(15 / 1_000_000);
    });

    it("通过剥离变体和厂商前缀匹配", () => {
      const pricing = getModelPricing("google/gemini-2.5-flash-latest");
      expect(pricing.input_cost_per_token).toBeCloseTo(0.15 / 1_000_000);
      expect(pricing.output_cost_per_token).toBeCloseTo(0.6 / 1_000_000);
    });

    it("关键字启发式匹配 (opus, haiku, flash, sonnet, deepseek)", () => {
      const opusPricing = getModelPricing("custom-private-opus-v2");
      expect(opusPricing.input_cost_per_token).toBeCloseTo(5 / 1_000_000);

      const haikuPricing = getModelPricing("company-internal-haiku");
      expect(haikuPricing.input_cost_per_token).toBeCloseTo(0.8 / 1_000_000);

      const deepseekPricing = getModelPricing("my-deepseek-custom-node");
      expect(deepseekPricing.input_cost_per_token).toBeCloseTo(0.27 / 1_000_000);
    });

    it("空输入或完全未知的模型返回默认兜底定价 (Sonnet 费率)", () => {
      expect(getModelPricing("")).toEqual(DEFAULT_MODEL_PRICING);

      const defaultPricing = getModelPricing("unknown-super-agi-9000");
      expect(defaultPricing.input_cost_per_token).toBeCloseTo(3 / 1_000_000);
      expect(defaultPricing.output_cost_per_token).toBeCloseTo(15 / 1_000_000);
    });
  });

  describe("fetchPricingCatalog 在线拉取与解析", () => {
    it("成功拉取 models.dev 数据 (对象结构) 并更新 catalog 与 localStorage", async () => {
      const mockModelsDevData = {
        anthropic: {
          id: "anthropic",
          name: "Anthropic",
          models: {
            "claude-3-7-sonnet": {
              id: "claude-3-7-sonnet",
              name: "Claude 3.7 Sonnet",
              cost: {
                input: 3.0,
                output: 15.0,
                cache_read: 0.3,
                cache_write: 3.75,
              },
            },
          },
        },
        openai: {
          id: "openai",
          name: "OpenAI",
          models: {
            "gpt-4.5-preview": {
              id: "gpt-4.5-preview",
              name: "GPT-4.5 Preview",
              cost: {
                input: 75.0,
                output: 150.0,
                cache_read: 37.5,
              },
            },
          },
        },
      };

      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce({
        ok: true,
        json: async () => mockModelsDevData,
      } as unknown as Response);

      const { catalog, updatedAt, isRefreshing, refreshError, fetchPricingCatalog, status, catalogItems } =
        usePricingCatalog();

      expect(isRefreshing.value).toBe(false);

      const fetchPromise = fetchPricingCatalog(true);
      expect(isRefreshing.value).toBe(true);

      await fetchPromise;
      expect(isRefreshing.value).toBe(false);
      expect(refreshError.value).toBe("");
      expect(updatedAt.value).not.toBeNull();

      // 验证全称与短别名已写入
      expect(catalog.value["anthropic/claude-3-7-sonnet"]).toBeDefined();
      expect(catalog.value["claude-3-7-sonnet"]).toBeDefined();
      expect(catalog.value["anthropic/claude-3-7-sonnet"].input_cost_per_token).toBeCloseTo(3 / 1_000_000);

      // 验证 localStorage 已持久化
      const savedCatalog = store[STORAGE_KEY_CATALOG];
      const savedUpdatedAt = store[STORAGE_KEY_UPDATED_AT];
      expect(savedCatalog).toBeDefined();
      expect(savedUpdatedAt).toBeDefined();

      // 验证 status 与 catalogItems
      expect(status.value.model_count).toBeGreaterThan(0);
      expect(status.value.updated_at).toBe(updatedAt.value);

      const item37 = catalogItems.value.find((i) => i.model === "claude-3-7-sonnet");
      expect(item37).toBeDefined();
      expect(item37?.provider).toBe("anthropic");
      expect(item37?.inputPer1M).toBe(3.0);
      expect(item37?.outputPer1M).toBe(15.0);
      expect(item37?.cacheWritePer1M).toBe(3.75);
      expect(item37?.cacheReadPer1M).toBe(0.3);

      // 再次非强制拉取且已有有效缓存时不发起网络请求
      fetchSpy.mockClear();
      await fetchPricingCatalog(false);
      expect(fetchSpy).not.toHaveBeenCalled();
    });

    it("支持解析数组结构的数据源", async () => {
      const mockArrayData = [
        {
          provider: "mistral",
          id: "mistral-large-2407",
          cost: {
            input: 2.0,
            output: 6.0,
          },
        },
      ];

      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce({
        ok: true,
        json: async () => mockArrayData,
      } as unknown as Response);

      const { catalog, fetchPricingCatalog, catalogItems } = usePricingCatalog();
      await fetchPricingCatalog(true);

      expect(catalog.value["mistral/mistral-large-2407"]).toBeDefined();
      expect(catalog.value["mistral-large-2407"]).toBeDefined();
      expect(catalog.value["mistral/mistral-large-2407"].input_cost_per_token).toBeCloseTo(2 / 1_000_000);

      const mistralItem = catalogItems.value.find((i) => i.model === "mistral-large-2407");
      expect(mistralItem).toBeDefined();
      expect(mistralItem?.provider).toBe("mistral");
    });

    it("网络请求失败时记录错误且保留既有目录，不中断程序", async () => {
      vi.spyOn(globalThis, "fetch").mockRejectedValueOnce(new Error("Failed to fetch models.dev"));

      const { catalog, fetchPricingCatalog, isRefreshing, refreshError } = usePricingCatalog();

      const initialKeyCount = Object.keys(catalog.value).length;
      expect(initialKeyCount).toBeGreaterThan(0);

      await fetchPricingCatalog(true);

      expect(isRefreshing.value).toBe(false);
      expect(refreshError.value).toContain("Failed to fetch models.dev");
      // 仍然保留默认目录
      expect(Object.keys(catalog.value).length).toBe(initialKeyCount);
    });

    it("HTTP 错误状态码时记录异常信息", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce({
        ok: false,
        status: 503,
        statusText: "Service Unavailable",
      } as unknown as Response);

      const { fetchPricingCatalog, refreshError } = usePricingCatalog();
      await fetchPricingCatalog(true);

      expect(refreshError.value).toContain("HTTP 503");
    });
  });

  describe("持久化缓存加载", () => {
    it("初始化时从 localStorage 加载持久化缓存与更新时间", () => {
      const mockCached = {
        "custom/my-model": {
          input_cost_per_token: 0.000001,
          output_cost_per_token: 0.000002,
        },
      };
      const mockTime = "2026-08-29T12:00:00.000Z";

      store[STORAGE_KEY_CATALOG] = JSON.stringify(mockCached);
      store[STORAGE_KEY_UPDATED_AT] = mockTime;

      resetPricingCatalogStore();
      const { catalog, updatedAt } = usePricingCatalog();

      expect(catalog.value["custom/my-model"]).toBeDefined();
      expect(catalog.value["custom/my-model"].input_cost_per_token).toBe(0.000001);
      expect(updatedAt.value).toBe(mockTime);
    });
  });

  describe("resolveProviderId 厂商 id 词表", () => {
    // 每条给「一个厂商 + 它的两种输入形状」：`provider/model` 的前缀（Pass 1 路径）
    // 与裸模型名（Pass 2 路径）。三处调用点共用这一个入口，两种形状必须归到同一个 slug——
    // 归不到一起就是同一厂商裂成两个下拉项，且不报错。
    const CASES: Array<{ id: string; inputs: string[] }> = [
      { id: "anthropic", inputs: ["anthropic", "claude-sonnet-4-6"] },
      { id: "openai", inputs: ["openai", "gpt-4o", "o1-mini", "o3-mini"] },
      { id: "google", inputs: ["google", "gemini-2.5-pro"] },
      { id: "deepseek", inputs: ["deepseek", "deepseek-chat"] },
      { id: "moonshot-kimi", inputs: ["moonshotai", "kimi-k2.5", "moonshot-v1-8k"] },
      { id: "zhipu", inputs: ["zhipuai", "glm-4", "zai"] },
      { id: "alibaba", inputs: ["alibaba", "qwen-max"] },
      { id: "minimax", inputs: ["minimax", "minimax-m2.5", "abab6.5s"] },
      { id: "stepfun", inputs: ["stepfun", "step-3.5-flash"] },
      { id: "baichuan", inputs: ["baichuan", "baichuan4"] },
      { id: "bytedance", inputs: ["volcengine", "bytedance", "doubao-pro"] },
      { id: "tencent", inputs: ["tencent", "hunyuan-pro"] },
      { id: "baidu", inputs: ["baidu", "qianfan", "ernie-4"] },
      { id: "siliconflow", inputs: ["siliconflow", "silicon"] },
      { id: "meta", inputs: ["meta", "llama-3.3-70b"] },
      { id: "mistral", inputs: ["mistral", "mistral-large-2407"] },
      { id: "groq", inputs: ["groq"] },
      { id: "xai", inputs: ["xai", "grok-4.5"] },
    ];

    it("18 个已知厂商的两种输入形状都归到同一个 slug", () => {
      for (const { id, inputs } of CASES) {
        for (const input of inputs) {
          expect(resolveProviderId(input), `${input} 应归到 ${id}`).toBe(id);
        }
      }
      // 表里必须覆盖整个词表，否则上面这轮会在词表缩小后静默变窄。
      expect(new Set(CASES.map((c) => c.id)).size).toBe(PROVIDER_IDS.length);
    });

    it("未知厂商沿用原有回退形态（首字母大写），不并入同一个桶", () => {
      // 未知厂商的 id 就是那个回退串本身：`availableProviders` 只滤掉大小写敏感的 "Other"，
      // 把未知厂商并成一个桶会让它们在下拉项里合并成一条。
      expect(resolveProviderId("unlisted-vendor")).toBe("Unlisted-vendor");
      expect(resolveProviderId("custom")).toBe("Custom");
      expect(isKnownProviderId("Unlisted-vendor")).toBe(false);
      // 阳性对照：同一形状的已知厂商不会被回退串顶替
      expect(resolveProviderId("glm-4")).toBe("zhipu");
    });
  });

  describe("providerDisplayName 显示出口", () => {
    it("已知 id 取语言包取值，未知 id 原样返回（不经过 t()）", () => {
      expect(providerDisplayName("zhipu")).toBe(t("dialogs.pricing.provider.zhipu"));
      expect(providerDisplayName("Unlisted-vendor")).toBe("Unlisted-vendor");
      // 未知 id 若走了 t()，缺键会让 vue-i18n 把 key path 渲染出来
      expect(providerDisplayName("Unlisted-vendor")).not.toContain("dialogs.pricing");
    });

    it("「当前会话」哨兵 id 不落在词表里，显示名取 providerCurrentSession", () => {
      expect(isKnownProviderId(CURRENT_SESSION_PROVIDER_ID)).toBe(false);
      expect(providerDisplayName(CURRENT_SESSION_PROVIDER_ID)).toBe(
        t("dialogs.pricing.providerCurrentSession")
      );
    });

    it("18 个 slug 在四份语言包里都有取值", () => {
      const packs: Array<[string, Record<string, any>]> = [
        ["en", enDialogs],
        ["zh", zhDialogs],
        ["ja", jaDialogs],
        ["de", deDialogs],
      ];
      for (const [lang, pack] of packs) {
        const provider = pack.pricing.provider as Record<string, unknown>;
        for (const id of PROVIDER_IDS) {
          expect(typeof provider[id], `${lang} 缺 dialogs.pricing.provider.${id}`).toBe("string");
        }
      }
    });
  });
});
