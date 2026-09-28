import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CliId, CliOption } from "../types/cli";
import { CLI_DEFINITIONS } from "../types/cli";
import type { UsageRecord } from "../types/session";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  failures: new Set<CliId>(),
  cliOptions: undefined as ReturnType<typeof ref<CliOption[]>> | undefined,
  cliSessionCounts: undefined as ReturnType<typeof ref<Record<string, number>>> | undefined,
  blockedFolders: undefined as ReturnType<typeof ref<string[]>> | undefined,
}));

const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});
vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
  callback(0);
  return 0;
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../composables/useBlockedFolders", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.blockedFolders = ref<string[]>([]);
  return {
    useBlockedFolders: () => ({
      blockedFolders: mocks.blockedFolders,
      loadBlockedFolders: vi.fn().mockResolvedValue(undefined),
      isBlocked: (p: string) =>
        (mocks.blockedFolders?.value ?? []).some((b: string) => p === b || p.startsWith(b + "/") || p.startsWith(b + "\\")),
    }),
    isPathBlocked: (p: string, list: string[]) =>
      list.some((b: string) => p === b || p.startsWith(b + "/") || p.startsWith(b + "\\")),
  };
});
vi.mock("../composables/useSessions", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  const options: CliOption[] = (["claude", "gemini", "workbuddy"] as CliId[]).map((id) => ({
    ...CLI_DEFINITIONS[id],
    hasSessions: true,
    hasBinary: true,
  }));
  mocks.cliOptions = ref(options);
  mocks.cliSessionCounts = ref({
    claude: 5,
    gemini: 3,
    workbuddy: 2,
  });
  return {
    useSessions: () => ({
      cliOptions: mocks.cliOptions,
      cliSessionCounts: mocks.cliSessionCounts,
    }),
  };
});

const { mount, flushPromises } = await import("@vue/test-utils");
const UsageDashboard = (await import("./UsageDashboard.vue")).default;
const { t } = await import("../i18n");

function makeUsageRecord(overrides: Partial<UsageRecord> = {}): UsageRecord {
  return {
    date: new Date().toISOString().slice(0, 10),
    model: "claude-sonnet-4-6",
    input_tokens: 1000,
    output_tokens: 500,
    cache_creation_tokens: 200,
    cache_read_tokens: 100,
    duration_ms: 1200,
    project: "seshbuddy-project",
    ...overrides,
  };
}

const mockUsageData: Record<string, UsageRecord[]> = {
  claude: [
    makeUsageRecord({ model: "claude-sonnet-4-6", input_tokens: 2000, output_tokens: 1000, project: "project-a" }),
  ],
  gemini: [
    makeUsageRecord({ model: "claude-haiku-4-5", input_tokens: 4000, output_tokens: 2000, project: "project-b" }),
  ],
};

import { resetUsageStatsStore, useUsageStats } from "../composables/useUsageStats";
import { resetPricingCatalogStore, usePricingCatalog } from "../composables/usePricingCatalog";
import PricingCatalogDialog from "./PricingCatalogDialog.vue";

beforeEach(() => {
  storage.clear();
  resetUsageStatsStore();
  resetPricingCatalogStore();
  mocks.invoke.mockReset();
  mocks.failures.clear();
  if (mocks.blockedFolders) mocks.blockedFolders.value = [];
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
    if (command === "get_usage_stats") {
      const cliId = args?.cliId as CliId;
      if (mocks.failures.has(cliId)) throw new Error(`${cliId} stats failed`);
      return mockUsageData[cliId] ?? [];
    }
    return undefined;
  });
});

function mountDashboard() {
  return mount(UsageDashboard, {
    global: {
      stubs: {
        SvgIcon: true,
        ChatAvatar: true,
      },
    },
  });
}

describe("UsageDashboard multi-CLI fan-out & chips", () => {
  it("defaults to mode 'all' and invokes get_usage_stats for each supported CLI", async () => {
    const wrapper = mountDashboard();
    await flushPromises();

    // claude and gemini support usage stats; workbuddy does not
    const calls = mocks.invoke.mock.calls
      .filter(([cmd]) => cmd === "get_usage_stats")
      .map(([, args]) => args.cliId);
    expect(calls.sort()).toEqual(["claude", "gemini"]);

    // "全部" chip and per-CLI chips exist
    const allChip = wrapper.find('[data-testid="usage-cli-all"]');
    expect(allChip.exists()).toBe(true);
    expect(allChip.classes()).toContain("active");

    const claudeChip = wrapper.find('[data-cli-id="claude"]');
    const geminiChip = wrapper.find('[data-cli-id="gemini"]');
    expect(claudeChip.exists()).toBe(true);
    expect(geminiChip.exists()).toBe(true);
    expect(claudeChip.classes()).toContain("active");
    expect(geminiChip.classes()).toContain("active");

    // workbuddy should not have a chip because supportsUsageStats is false
    expect(wrapper.find('[data-cli-id="workbuddy"]').exists()).toBe(false);

    // Displays session counts from cliSessionCounts
    expect(claudeChip.text()).toContain("5");
    expect(geminiChip.text()).toContain("3");

    // Totals should aggregate both claude and gemini
    // claude tokens: 2000+1000+200+100 = 3300
    // gemini tokens: 4000+2000+200+100 = 6300
    // total = 9600 => formatTokens: "9.6K"
    expect(wrapper.text()).toContain("9.6K");
  });

  it("toggles individual CLI chips and refetches only selected CLI", async () => {
    const wrapper = mountDashboard();
    await flushPromises();

    mocks.invoke.mockClear();

    // Click gemini chip to deselect it
    const geminiChip = wrapper.find('[data-cli-id="gemini"]');
    await geminiChip.trigger("click");
    await flushPromises();

    // "全部" chip is no longer active
    expect(wrapper.find('[data-testid="usage-cli-all"]').classes()).not.toContain("active");
    expect(wrapper.find('[data-cli-id="gemini"]').classes()).not.toContain("active");
    expect(wrapper.find('[data-cli-id="claude"]').classes()).toContain("active");

    // get_usage_stats called for claude only
    const calls = mocks.invoke.mock.calls
      .filter(([cmd]) => cmd === "get_usage_stats")
      .map(([, args]) => args.cliId);
    expect(calls).toEqual(["claude"]);

    // Total tokens should now only be claude's 3300 => "3.3K"
    expect(wrapper.text()).toContain("3.3K");
  });

  it("toggles '全部' chip between mode all and empty custom mode", async () => {
    const wrapper = mountDashboard();
    await flushPromises();

    const allChip = wrapper.find('[data-testid="usage-cli-all"]');
    // Click "全部" when all selected -> toggles to empty custom
    await allChip.trigger("click");
    await flushPromises();

    expect(allChip.classes()).not.toContain("active");
    expect(wrapper.find('[data-cli-id="claude"]').classes()).not.toContain("active");
    expect(wrapper.find('[data-cli-id="gemini"]').classes()).not.toContain("active");
    expect(wrapper.text()).toContain(t("session.usageDashboard.empty"));

    // Click "全部" again -> toggles back to all
    await allChip.trigger("click");
    await flushPromises();

    expect(allChip.classes()).toContain("active");
    expect(wrapper.find('[data-cli-id="claude"]').classes()).toContain("active");
    expect(wrapper.find('[data-cli-id="gemini"]').classes()).toContain("active");
    expect(wrapper.text()).toContain("9.6K");
  });

  it("handles partial failures gracefully by keeping available records", async () => {
    mocks.failures.add("gemini");
    const wrapper = mountDashboard();
    await flushPromises();

    // Should not show error view, claude's records are rendered
    expect(wrapper.text()).not.toContain(t("session.usageDashboard.loadError"));
    expect(wrapper.text()).toContain("3.3K"); // claude's tokens
  });

  it("preserves last known data when a previously succeeded CLI fails on refresh", async () => {
    const wrapper = mountDashboard();
    await flushPromises();
    expect(wrapper.text()).toContain("9.6K");

    // Now gemini fails, but claude succeeds
    mocks.failures.add("gemini");
    mocks.invoke.mockClear();

    // Toggle claude chip off then on to trigger fetch
    await wrapper.find('[data-cli-id="claude"]').trigger("click");
    await flushPromises();
    await wrapper.find('[data-cli-id="claude"]').trigger("click");
    await flushPromises();

    // Gemini cached data should still be retained
    expect(wrapper.text()).toContain("9.6K");
  });

  it("persists filter changes to localStorage and restores them on mount", async () => {
    const wrapper = mountDashboard();
    await flushPromises();

    // Deselect claude
    await wrapper.find('[data-cli-id="claude"]').trigger("click");
    await flushPromises();

    expect(storage.get("seshbuddy-usage-cli-filter")).toBe(
      JSON.stringify({ mode: "custom", cliIds: ["gemini"] }),
    );

    wrapper.unmount();

    // Mount a new instance, should restore persisted filter
    mocks.invoke.mockClear();
    const wrapper2 = mountDashboard();
    await flushPromises();

    const calls = mocks.invoke.mock.calls
      .filter(([cmd]) => cmd === "get_usage_stats")
      .map(([, args]) => args.cliId);
    expect(calls).toEqual(["gemini"]);
    expect(wrapper2.find('[data-testid="usage-cli-all"]').classes()).not.toContain("active");
    expect(wrapper2.find('[data-cli-id="gemini"]').classes()).toContain("active");
    expect(wrapper2.find('[data-cli-id="claude"]').classes()).not.toContain("active");
  });

  it("emits close when clicking the close button", async () => {
    const wrapper = mountDashboard();
    await flushPromises();

    await wrapper.find(".close-btn").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  describe("已屏蔽文件夹用量统计可勾选", () => {
    it("无屏蔽文件夹时不显示屏蔽筛选区", async () => {
      mocks.blockedFolders!.value = [];
      const wrapper = mountDashboard();
      await flushPromises();

      expect(wrapper.find('[data-testid="usage-blocked-folders-section"]').exists()).toBe(false);
      expect(wrapper.text()).toContain("9.6K");
    });

    it("有屏蔽文件夹时默认不勾选（排除用量），勾选后纳入统计，支持全选/全不选", async () => {
      mocks.blockedFolders!.value = ["project-a"];
      const wrapper = mountDashboard();
      await flushPromises();

      // 显示屏蔽筛选区域
      const section = wrapper.find('[data-testid="usage-blocked-folders-section"]');
      expect(section.exists()).toBe(true);
      expect(section.text()).toContain("project-a");

      // 默认未勾选：project-a（3.3K）被排除，只剩 gemini 的 project-b（6.3K）
      expect(wrapper.text()).toContain("6.3K");

      const checkbox = section.find<HTMLInputElement>('input[type="checkbox"]');
      expect(checkbox.element.checked).toBe(false);

      // 单独勾选 project-a
      await checkbox.setChecked(true);
      await wrapper.vm.$nextTick();

      // 纳入统计：总计 9.6K
      expect(wrapper.text()).toContain("9.6K");

      // 全选/全不选 按钮
      const toggleAllBtn = section.find('[data-testid="toggle-all-blocked"]');
      expect(toggleAllBtn.text()).toBe(t("session.usageDashboard.deselectAll"));

      // 点击全不选
      await toggleAllBtn.trigger("click");
      await wrapper.vm.$nextTick();
      expect(wrapper.text()).toContain("6.3K");
      expect(toggleAllBtn.text()).toBe(t("app.batch.selectAll"));

      // 点击全选
      await toggleAllBtn.trigger("click");
      await wrapper.vm.$nextTick();
      expect(wrapper.text()).toContain("9.6K");
      expect(toggleAllBtn.text()).toBe(t("session.usageDashboard.deselectAll"));
    });
  });

  describe("Window Control & Maximize Mode", () => {
    it("reads initial maximize state from localStorage and toggles maximize class", async () => {
      storage.set("seshbuddy-usage-maximized", "true");
      const wrapper = mountDashboard();
      await flushPromises();

      const windowEl = wrapper.find(".usage-window");
      expect(windowEl.classes()).toContain("is-maximized");

      const maxBtn = wrapper.find(".maximize-btn");
      expect(maxBtn.attributes("title")).toBe(t("session.usageDashboard.restore"));

      // Click to restore normal
      await maxBtn.trigger("click");
      expect(windowEl.classes()).not.toContain("is-maximized");
      expect(storage.get("seshbuddy-usage-maximized")).toBe("false");
      expect(maxBtn.attributes("title")).toBe(t("session.usageDashboard.maximize"));

      // Click again to maximize
      await maxBtn.trigger("click");
      expect(windowEl.classes()).toContain("is-maximized");
      expect(storage.get("seshbuddy-usage-maximized")).toBe("true");
    });
  });

  describe("SWR Caching & Refresh", () => {
    it("mounts without skeleton flash when cached data is already in store", async () => {
      // Pre-populate store
      const store = useUsageStats();
      await store.fetchUsage(["claude", "gemini"]);
      expect(store.lastFetchedAt.value).not.toBeNull();

      mocks.invoke.mockClear();

      // Mount dashboard: should render data immediately without loading skeleton
      const wrapper = mountDashboard();
      expect(wrapper.find(".usage-skeleton").exists()).toBe(false);
      expect(wrapper.find(".usage-data-view").exists()).toBe(true);
      expect(wrapper.text()).toContain("9.6K");
    });

    it("triggers force refetch when clicking the refresh button", async () => {
      const wrapper = mountDashboard();
      await flushPromises();

      mocks.invoke.mockClear();
      const refreshBtn = wrapper.find(".refresh-btn");
      expect(refreshBtn.exists()).toBe(true);

      await refreshBtn.trigger("click");
      await flushPromises();

      const calls = mocks.invoke.mock.calls
        .filter(([cmd]) => cmd === "get_usage_stats")
        .map(([, args]) => args.cliId);
      expect(calls.sort()).toEqual(["claude", "gemini"]);
    });
  });

  describe("Bidirectional Drilldown Cards", () => {
    it("renders model breakdown card and toggles project drilldown sublist", async () => {
      const wrapper = mountDashboard();
      await flushPromises();

      const modelCard = wrapper.find('[data-testid="model-breakdown-card"]');
      expect(modelCard.exists()).toBe(true);
      expect(modelCard.text()).toContain("claude-sonnet-4-6");
      expect(modelCard.text()).toContain("claude-haiku-4-5");

      // Sublist initially collapsed
      expect(wrapper.find('[data-testid="model-sublist-claude-sonnet-4-6"]').exists()).toBe(false);

      // Click to expand sonnet model
      const sonnetRow = wrapper.find('[data-testid="model-row-claude-sonnet-4-6"]');
      await sonnetRow.trigger("click");
      await wrapper.vm.$nextTick();

      const sublist = wrapper.find('[data-testid="model-sublist-claude-sonnet-4-6"]');
      expect(sublist.exists()).toBe(true);
      expect(sublist.text()).toContain("project-a");
      expect(sublist.text()).toContain("100.0%");

      // Click again to collapse
      await sonnetRow.trigger("click");
      await wrapper.vm.$nextTick();
      expect(wrapper.find('[data-testid="model-sublist-claude-sonnet-4-6"]').exists()).toBe(false);
    });

    it("renders project breakdown card and toggles model drilldown sublist", async () => {
      const wrapper = mountDashboard();
      await flushPromises();

      const projectCard = wrapper.find('[data-testid="project-breakdown-card"]');
      expect(projectCard.exists()).toBe(true);
      expect(projectCard.text()).toContain("project-a");
      expect(projectCard.text()).toContain("project-b");

      // Sublist initially collapsed
      expect(wrapper.find('[data-testid="project-sublist-project-a"]').exists()).toBe(false);

      // Click to expand project-a
      const projARow = wrapper.find('[data-testid="project-row-project-a"]');
      await projARow.trigger("click");
      await wrapper.vm.$nextTick();

      const sublist = wrapper.find('[data-testid="project-sublist-project-a"]');
      expect(sublist.exists()).toBe(true);
      expect(sublist.text()).toContain("claude-sonnet-4-6");
      expect(sublist.text()).toContain("100.0%");

      // Click again to collapse
      await projARow.trigger("click");
      await wrapper.vm.$nextTick();
      expect(wrapper.find('[data-testid="project-sublist-project-a"]').exists()).toBe(false);
    });

    it("limits items to 10 by default and toggles more when clicking more button", async () => {
      // Mock 12 distinct models
      // ⚠️ 日期必须**相对今天**取：组件按默认时间窗（`30d`）过滤，
      // 写死一个绝对日期会在它超出 30 天窗口的那天起让这条用例永久失败。
      const today = new Date().toISOString().slice(0, 10);
      const multiRecords = Array.from({ length: 12 }, (_, i) => ({
        date: today,
        model: `model-${String(i + 1).padStart(2, "0")}`,
        input_tokens: 1000 * (12 - i),
        output_tokens: 500,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 1000,
        project: `/proj-${String(i + 1).padStart(2, "0")}`,
      }));
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
        if (command === "get_usage_stats") {
          const cliId = args?.cliId as CliId;
          if (cliId === "claude") return multiRecords;
          return [];
        }
        return undefined;
      });

      const wrapper = mountDashboard();
      await flushPromises();

      const modelCard = wrapper.find('[data-testid="model-breakdown-card"]');
      expect(modelCard.findAll(".drilldown-group")).toHaveLength(10);

      const moreBtn = wrapper.find('[data-testid="toggle-more-models"]');
      expect(moreBtn.exists()).toBe(true);
      expect(moreBtn.text()).toContain(t("session.usageDashboard.expandModels", { count: 2 }));

      // Click to expand
      await moreBtn.trigger("click");
      await wrapper.vm.$nextTick();
      expect(modelCard.findAll(".drilldown-group")).toHaveLength(12);
      expect(moreBtn.text()).toContain(t("session.usageDashboard.collapseModels"));
    });
  });

  describe("Pricing Catalog Dialog Integration", () => {
    it("renders pricing catalog button in toolbar", async () => {
      const wrapper = mountDashboard();
      await flushPromises();

      const btn = wrapper.find('[data-testid="open-pricing-catalog-btn"]');
      expect(btn.exists()).toBe(true);
      expect(btn.text()).toContain(t("session.usageDashboard.pricingCatalog"));
    });

    it("opens PricingCatalogDialog when clicking pricing button and closes it on close event", async () => {
      const wrapper = mountDashboard();
      await flushPromises();

      // Initially closed
      expect(wrapper.findComponent(PricingCatalogDialog).exists()).toBe(false);

      // Open dialog
      const btn = wrapper.find('[data-testid="open-pricing-catalog-btn"]');
      await btn.trigger("click");
      await wrapper.vm.$nextTick();

      const dialog = wrapper.findComponent(PricingCatalogDialog);
      expect(dialog.exists()).toBe(true);

      // Emit close from dialog
      await dialog.vm.$emit("close");
      await wrapper.vm.$nextTick();

      expect(wrapper.findComponent(PricingCatalogDialog).exists()).toBe(false);
    });

    it("automatically recalculates total cost and drilldowns when pricing catalog updates", async () => {
      const wrapper = mountDashboard();
      await flushPromises();

      const initialText = wrapper.text();

      // Now update pricing catalog with 100x higher price for sonnet-4-6
      const { catalog } = usePricingCatalog();
      catalog.value = {
        ...catalog.value,
        "claude-sonnet-4-6": {
          input_cost_per_token: 300 / 1_000_000,
          output_cost_per_token: 1500 / 1_000_000,
          cache_creation_input_token_cost: 375 / 1_000_000,
          cache_read_input_token_cost: 30 / 1_000_000,
        },
      };
      await wrapper.vm.$nextTick();

      // Cost should immediately reflect the updated pricing
      const updatedText = wrapper.text();
      expect(updatedText).not.toEqual(initialText);
      expect(updatedText).toContain("$2.1");
    });
  });
});
