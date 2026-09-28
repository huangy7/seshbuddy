import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import PricingCatalogDialog from "./PricingCatalogDialog.vue";
import {
  DEFAULT_MODEL_PRICING,
  resetPricingCatalogStore,
  usePricingCatalog,
  CURRENT_SESSION_PROVIDER_ID,
} from "../composables/usePricingCatalog";
import { t, setLocale } from "../i18n";

describe("PricingCatalogDialog.vue", () => {
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
    // locale 是全局状态，用例跑在基准语言上，切换过的用例必须还原。
    setLocale("en");
  });

  function mountDialog() {
    return mount(PricingCatalogDialog, {
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
      attachTo: document.body,
    });
  }

  it("renders header with title, model count badge and update info", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    expect(wrapper.text()).toContain(t("dialogs.pricing.title"));
    const countBadge = wrapper.find(".model-count-badge");
    expect(countBadge.exists()).toBe(true);
    // 模型数量随内置目录变化，从 store 取当前值；文案走 t(...) 而非写死英文，
    // 与全仓测试一致——断言源文案等价于断言契约，不随翻译改动而碎。
    const { status } = usePricingCatalog();
    expect(status.value.model_count).toBeGreaterThan(0);
    expect(countBadge.text()).toBe(
      t("dialogs.pricing.modelCount", { count: status.value.model_count })
    );
    expect(wrapper.text()).toContain("models.dev");
    wrapper.unmount();
  });

  it("renders pricing table rows with model, provider, and formatted rates", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);

    // Default catalog includes claude-sonnet-4-6
    expect(wrapper.text()).toContain("claude-sonnet-4-6");
    expect(wrapper.text()).toContain("Anthropic");
    wrapper.unmount();
  });

  it("filters models by search query matching model name, provider or key", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const searchInput = wrapper.find('input[type="text"]');
    expect(searchInput.exists()).toBe(true);

    // Search for deepseek
    await searchInput.setValue("deepseek");
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.text().toLowerCase()).toContain("deepseek");
    }

    // Search for non-existent model
    await searchInput.setValue("non-existent-model-xyz-12345");
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll("tbody tr").length).toBe(0);
    expect(wrapper.text()).toContain(t("dialogs.pricing.emptyTitle"));
    wrapper.unmount();
  });

  // 搜索比的是**显示名**而不是身份 id：用户看见的是「Zhipu / 智谱 GLM」，
  // 他搜的是「智谱」。只按 id 匹配的话，这条搜索会静默变空（过滤看起来只是「没结果」）。
  it("搜索命中厂商的显示名（中文品牌名也搜得到）", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const searchInput = wrapper.find('input[type="text"]');
    await searchInput.setValue("智谱");
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.text()).toContain("智谱");
    }

    // 阳性对照：同一个输入框里搜一个不存在的串仍然是空结果，
    // 证明上面的「有结果」不是「过滤器根本没生效」。
    await searchInput.setValue("no-such-provider-or-model-xyz");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll("tbody tr").length).toBe(0);
    wrapper.unmount();
  });

  it("filters models by provider select dropdown", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const providerSelect = wrapper.findComponent('[data-testid="provider-select"]');
    expect(providerSelect.exists()).toBe(true);

    // 选中项存的是厂商 id；界面上显示的是语言包里的厂商名。
    await providerSelect.vm.$emit("update:modelValue", "anthropic");
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.text()).toContain(t("dialogs.pricing.provider.anthropic"));
    }

    // Reset to all
    await providerSelect.vm.$emit("update:modelValue", "all");
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll("tbody tr").length).toBeGreaterThan(rows.length);
    wrapper.unmount();
  });

  it("filters to used models when usedModels prop is passed and supports scope switching", async () => {
    const wrapper = mount(PricingCatalogDialog, {
      props: {
        usedModels: ["claude-3-5-sonnet", "deepseek-v3"],
      },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    await flushPromises();

    const usedTab = wrapper.find('[data-testid="scope-used"]');
    expect(usedTab.exists()).toBe(true);
    expect(usedTab.classes()).toContain("active");

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBe(2);
    expect(wrapper.text()).toContain("claude-3-5-sonnet");
    expect(wrapper.text()).toContain("deepseek-v3");

    // Switch to all scope
    const allTab = wrapper.find('[data-testid="scope-all"]');
    expect(allTab.exists()).toBe(true);
    await allTab.trigger("click");
    await wrapper.vm.$nextTick();

    expect(allTab.classes()).toContain("active");
    expect(wrapper.findAll("tbody tr").length).toBeGreaterThan(2);
    wrapper.unmount();
  });

  it("emits close when clicking close button, overlay backdrop or pressing Escape", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    // 1. Close button
    const closeBtn = wrapper.find(".close-btn");
    expect(closeBtn.exists()).toBe(true);
    await closeBtn.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);

    // 2. Overlay backdrop click
    const overlay = wrapper.find(".modal-backdrop");
    await overlay.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(2);

    // 3. Escape keydown
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(wrapper.emitted("close")).toHaveLength(3);

    wrapper.unmount();
  });

  it("calls fetchPricingCatalog(true) when clicking refresh button", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce({
      ok: true,
      json: async () => ({}),
    } as unknown as Response);

    const wrapper = mountDialog();
    await flushPromises();

    const refreshBtn = wrapper.find(".refresh-btn");
    expect(refreshBtn.exists()).toBe(true);
    await refreshBtn.trigger("click");
    await flushPromises();

    expect(fetchSpy).toHaveBeenCalledWith("https://models.dev/api.json");
    wrapper.unmount();
  });

  it("displays loading/spinning state when isRefreshing is true", async () => {
    const store = usePricingCatalog();
    store.isRefreshing.value = true;

    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    const refreshBtn = wrapper.find(".refresh-btn");
    expect(refreshBtn.classes()).toContain("is-spinning");
    expect(refreshBtn.attributes("disabled")).toBeDefined();

    store.isRefreshing.value = false;
    wrapper.unmount();
  });

  it("clears search input when clicking clear button", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const searchInput = wrapper.find('input[type="text"]');
    await searchInput.setValue("claude");
    await wrapper.vm.$nextTick();

    const clearBtn = wrapper.find(".clear-search-btn");
    expect(clearBtn.exists()).toBe(true);

    await clearBtn.trigger("click");
    await wrapper.vm.$nextTick();

    expect((searchInput.element as HTMLInputElement).value).toBe("");
    expect(wrapper.find(".clear-search-btn").exists()).toBe(false);
    wrapper.unmount();
  });

  it("displays refresh error banner when refreshError is set", async () => {
    const store = usePricingCatalog();
    store.refreshError.value = "Network connection timeout";

    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    const errorBanner = wrapper.find(".refresh-error-banner");
    expect(errorBanner.exists()).toBe(true);
    expect(errorBanner.text()).toContain("Network connection timeout");

    store.refreshError.value = "";
    wrapper.unmount();
  });

  // 「当前会话」哨兵项的身份是 id（不随语言变），显示名才是随语言变的那一份。
  // 这个对话框能挺过一次「进设置改语言」的往返（showSettings 与 showUsage 互不关闭），
  // 故语言切换后按它过滤的结果与它的标签都必须有守卫。
  it("按「当前会话」哨兵过滤在切换语言后仍有结果，标签跟随语言", async () => {
    const wrapper = mount(PricingCatalogDialog, {
      props: { usedModels: ["unlisted-model-xyz"] },
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    const providerSelect = wrapper.findComponent('[data-testid="provider-select"]');
    const sentinelOption = () =>
      (providerSelect.props("options") as Array<{ value: string; label: string }>).find(
        (opt) => opt.value === CURRENT_SESSION_PROVIDER_ID,
      );
    expect(sentinelOption()?.label).toBe(t("dialogs.pricing.providerCurrentSession"));

    await providerSelect.vm.$emit("update:modelValue", CURRENT_SESSION_PROVIDER_ID);
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll("tbody tr").length).toBe(1);

    setLocale("zh");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll("tbody tr").length).toBe(1);
    expect(sentinelOption()?.label).toBe(t("dialogs.pricing.providerCurrentSession"));

    wrapper.unmount();
  });

  // 厂商的身份是 id（词表见 usePricingCatalog 的 PROVIDER_IDS），显示名在语言包里随语言变。
  // 拆开之前厂商名同时是显示文案与数据标识（去重键、下拉项 value、过滤比较都用它），
  // 语言包里那六个取值一旦被改动，对应厂商会掉出主流排序、或同一家被拆成两条下拉项——
  // 两种失败都是静默的。现在这层耦合已由结构消除（三处调用点共用 resolveProviderId），
  // 下面的断言守卫的是「同一厂商的每条产出路径都落到同一个 id」：只给某一条路径写错 slug
  // （或让某条路径漏掉一个分支）就会红。
  //
  // 与拆开前相比这条断言更强：ByteDance / Baidu / Tencent 的裸模型名过去整条落到 Other 被丢弃，
  // 「产出集合恰好一个取值」对它们只是靠「只有前缀路径产出」成立的；词表补齐后两条路径都产出。
  const PROVIDER_PROBES: Array<{ id: string; withPrefix: string; bare: string }> = [
    { id: "zhipu", withPrefix: "zhipuai/glm-5", bare: "glm-5" },
    { id: "alibaba", withPrefix: "alibaba/qwen-max", bare: "qwen-max" },
    { id: "stepfun", withPrefix: "stepfun/step-3.5-flash", bare: "step-3.5-flash" },
    { id: "bytedance", withPrefix: "bytedance/doubao-pro", bare: "doubao-pro" },
    { id: "baidu", withPrefix: "baidu/ernie-4", bare: "ernie-4" },
    { id: "tencent", withPrefix: "tencent/hunyuan-pro", bare: "hunyuan-pro" },
  ];

  describe("厂商身份（id）在每条产出路径上一致", () => {
    it("每个厂商的每条产出路径都得出同一个 id", () => {
      const store = usePricingCatalog();
      for (const { id, withPrefix, bare } of PROVIDER_PROBES) {
        // 带 `provider/` 前缀的键与裸模型名在数据源里是两条独立路径（Pass 1 与 Pass 2）。
        // 断言产出集合**恰好**是那一个 id：只改动其中一条路径，同一家厂商就会在界面上
        // 裂成两条下拉项。
        store.catalog.value = {
          [withPrefix]: DEFAULT_MODEL_PRICING,
          [bare]: DEFAULT_MODEL_PRICING,
        };
        const produced = Array.from(
          new Set(store.catalogItems.value.map((item) => item.provider)),
        );
        expect(
          produced,
          `${id}：数据源产出的厂商 id 必须只有「${id}」一种，实际为 ${produced.join(" / ")}`,
        ).toEqual([id]);
      }
    });

    it("同一厂商经三条输入路径产出同一个 id", async () => {
      const store = usePricingCatalog();

      // 路径 ①②：数据源里的 `provider/model` 前缀键与裸模型名。
      // Pass 1 与 Pass 2 共用 resolveProviderId，故裸模型名会被去重掉，只产出一条。
      store.catalog.value = {
        "zhipuai/glm-4": DEFAULT_MODEL_PRICING,
        "glm-4": DEFAULT_MODEL_PRICING,
      };
      expect(store.catalogItems.value.map((item) => item.provider)).toEqual(["zhipu"]);

      // 路径 ③：usedItems 兜底链只在模型不在目录里时才走到（`found` 未命中），故先把目录清空。
      store.catalog.value = {};
      const wrapper = mount(PricingCatalogDialog, {
        props: { usedModels: ["glm-4"] },
        global: { stubs: { SvgIcon: true } },
      });
      await flushPromises();

      const options = wrapper
        .findComponent('[data-testid="provider-select"]')
        .props("options") as Array<{ value: string; label: string }>;
      expect(options.filter((opt) => opt.value !== "all").map((opt) => opt.value)).toEqual(["zhipu"]);
      // 显示出口：value 是 id，label 才是语言包里那一个取值。
      expect(options.find((opt) => opt.value === "zhipu")?.label).toBe(
        t("dialogs.pricing.provider.zhipu"),
      );
      wrapper.unmount();
    });
  });
});
