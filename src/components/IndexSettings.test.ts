import { beforeEach, describe, expect, it, vi } from "vitest";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";
import { formatRelativeTime } from "../utils/format";
import { t } from "../i18n";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  ask: vi.fn(),
  refresh: vi.fn(async () => undefined),
  searchIndexProgress: undefined as ReturnType<typeof import("vue")["ref"]> | undefined,
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

const cliOptions: CliOption[] = (Object.keys(CLI_DEFINITIONS) as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: id === "claude" || id === "codex",
  hasBinary: id === "claude" || id === "codex",
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (...args: unknown[]) => mocks.ask(...args),
  message: vi.fn(async () => undefined),
  open: vi.fn(),
}));
vi.mock("../composables/useSessions", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.searchIndexProgress = ref(null);
  return {
    useSessions: () => ({
      cliOptions: ref(cliOptions),
      searchIndexProgress: mocks.searchIndexProgress,
      refresh: mocks.refresh,
    }),
  };
});

const { mount, flushPromises } = await import("@vue/test-utils");
const IndexSettings = (await import("./IndexSettings.vue")).default;

const STATS = {
  sessionCount: 5,
  dbSizeBytes: 100,
  searchDocCount: 5,
  searchIndexBytes: 1000,
  lastUpdatedMs: null,
};

function mountCliMode() {
  return mount(IndexSettings, {
    props: { initialCliId: "claude", mode: "cli" },
    global: {
      stubs: {
        SvgIcon: true,
        ElegantSelect: { props: ["modelValue", "options"], template: "<select></select>" },
      },
    },
  });
}

function clearButton(wrapper: ReturnType<typeof mountCliMode>) {
  const btn = wrapper
    .findAll("button")
    .find((b) => b.text().includes(t("settings.index.clear")));
  if (!btn) throw new Error("clear button not found");
  return btn;
}

describe("IndexSettings 索引统计口径", () => {
  beforeEach(() => {
    storage.clear();
    mocks.invoke.mockReset();
    mocks.ask.mockReset();
    mocks.searchIndexProgress!.value = null;
    mocks.invoke.mockImplementation(async (command: string, args: any) => {
      if (command === "get_index_stats") {
        if (args?.cliId === "codex") {
          return { sessionCount: 2, dbSizeBytes: 50, searchDocCount: 3, searchIndexBytes: 1000, lastUpdatedMs: null };
        }
        return STATS; // claude: 5 会话 / 5 文档 / 1000 B
      }
      return undefined;
    });
  });

  it("全局卡聚合展示共享索引大小与全 CLI 文档总数", async () => {
    const wrapper = mount(IndexSettings, {
      props: { initialCliId: "claude", mode: "global" },
      global: {
        stubs: {
          SvgIcon: true,
          ElegantSelect: { props: ["modelValue", "options"], template: "<select></select>" },
        },
      },
    });
    await flushPromises();

    // 共享目录 1000 B（各 CLI 相同，取一份）· 文档 5 + 3 = 8
    expect(wrapper.text()).toContain(t("settings.index.sharedIndexTitle"));
    expect(wrapper.text()).toContain(
      t("settings.index.sharedIndexStats", { size: "1000 B", count: "8" }),
    );
  });

  it("CLI 卡无索引文档时统计行显示 —", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_index_stats") {
        return { sessionCount: 0, dbSizeBytes: 0, searchDocCount: 0, searchIndexBytes: 179000000, lastUpdatedMs: Date.now() };
      }
      return undefined;
    });
    const wrapper = mountCliMode();
    await flushPromises();

    const line = wrapper.find(".status-line");
    expect(line.exists()).toBe(true);
    expect(line.text()).toBe("—");
  });

  it("CLI 卡有索引文档时统计行展示对话数、文档数与更新时间", async () => {
    const now = Date.now();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_index_stats") {
        return { ...STATS, lastUpdatedMs: now };
      }
      return undefined;
    });
    const wrapper = mountCliMode();
    await flushPromises();

    const line = wrapper.find(".status-line");
    expect(line.text()).toBe(
      t("settings.index.statsLine", {
        sessions: "5",
        docs: "5",
        updated: formatRelativeTime(new Date(now).toISOString()),
      }),
    );
  });
});

describe("IndexSettings v-model:cliId", () => {
  beforeEach(() => {
    storage.clear();
    mocks.invoke.mockReset();
    mocks.searchIndexProgress!.value = null;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_index_stats") return STATS;
      return undefined;
    });
  });

  it("挂载时内部解析值与 prop 不一致 → 立即向上 emit 解析值", async () => {
    const wrapper = mount(IndexSettings, {
      props: { cliId: "dsh", mode: "cli" },
      global: {
        stubs: {
          SvgIcon: true,
          ElegantSelect: { props: ["modelValue", "options"], template: "<select></select>" },
        },
      },
    });
    await flushPromises();

    const emitted = wrapper.emitted("update:cliId");
    expect(emitted).toBeTruthy();
    // 解析值必为有会话的 CLI（claude 或 codex，取决于 resolveFeatureCliId 的优先级）
    expect(["claude", "codex"]).toContain(emitted![0][0]);
  });

  it("内部切换向外 emit，prop 变化向内同步", async () => {
    const wrapper = mount(IndexSettings, {
      props: { cliId: "claude", mode: "cli" },
      global: {
        stubs: {
          SvgIcon: true,
          ElegantSelect: { props: ["modelValue", "options"], template: "<select></select>" },
        },
      },
    });
    await flushPromises();

    // 向外：用户在下拉中切换 CLI
    await wrapper
      .findComponent('[data-testid="index-cli-select"]')
      .vm.$emit("update:modelValue", "codex");
    await flushPromises();
    expect(wrapper.emitted("update:cliId")).toEqual([["codex"]]);

    // 向内：父级改变 prop（先模拟 v-model 回写 codex，再切回 claude，确保 prop 真实变化）
    mocks.invoke.mockClear();
    await wrapper.setProps({ cliId: "codex" });
    await flushPromises();
    await wrapper.setProps({ cliId: "claude" });
    await flushPromises();
    expect(mocks.invoke).toHaveBeenCalledWith("get_index_stats", { cliId: "claude" });
  });
});

describe("IndexSettings 清除索引竞态", () => {
  beforeEach(() => {
    storage.clear();
    mocks.invoke.mockReset();
    mocks.ask.mockReset();
    mocks.refresh.mockClear();
    mocks.searchIndexProgress!.value = null;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_index_stats") return STATS;
      return undefined;
    });
  });

  it("空闲时确认清除 → 正常执行 clear_session_index", async () => {
    mocks.ask.mockResolvedValue(true);
    const wrapper = mountCliMode();
    await flushPromises();

    await clearButton(wrapper).trigger("click");
    await flushPromises();

    expect(mocks.ask).toHaveBeenCalled();
    expect(mocks.invoke).toHaveBeenCalledWith("clear_session_index", { cliId: "claude" });
  });

  it("确认弹窗期间其他索引任务激活 → 确认后中止清除", async () => {
    let resolveAsk!: (value: boolean) => void;
    mocks.ask.mockImplementation(
      () => new Promise<boolean>((resolve) => (resolveAsk = resolve)),
    );
    const wrapper = mountCliMode();
    await flushPromises();

    // 1. 用户点击「清除」，确认弹窗打开（按钮此刻未禁用）
    await clearButton(wrapper).trigger("click");
    expect(mocks.ask).toHaveBeenCalled();

    // 2. 弹窗期间，另一个索引任务（如全局重建/后台同步）激活
    mocks.searchIndexProgress!.value = {
      cliId: "claude",
      phase: "scanning",
      current: 1,
      total: 2,
    };

    // 3. 用户确认弹窗
    resolveAsk(true);
    await flushPromises();

    // 4. 不应再下发清除命令
    expect(
      mocks.invoke.mock.calls.some(([cmd]) => cmd === "clear_session_index"),
    ).toBe(false);
  });
});
