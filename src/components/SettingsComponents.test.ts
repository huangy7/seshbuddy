// 设置页各分区组件的聚合测试：ArchivedSessionsSettings / ArchiveRetentionSettings /
// CacheMaintenanceSettings / IntegrationSettings / DataIndexSettings。
// 这些组件共享同一套 CLI 上下文 mock（见下方 mocks），故合并为一个测试文件。
import { nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  currentCliId: undefined as ReturnType<typeof ref<CliId>> | undefined,
  visibleCliIds: undefined as ReturnType<typeof ref<CliId[]>> | undefined,
  setCliDataDirOverride: vi.fn(async () => undefined),
  setSkipPermissions: vi.fn(),
  registerContextMenu: vi.fn(async () => undefined),
  unregisterContextMenu: vi.fn(async () => undefined),
  refresh: vi.fn(async () => undefined),
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
  ask: vi.fn(async () => true),
  message: vi.fn(async () => undefined),
  open: vi.fn(),
}));
vi.mock("../composables/useSessions", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.currentCliId = ref<CliId>("claude");
  mocks.visibleCliIds = ref<CliId[]>(["claude"]);
  return {
    useSessions: () => ({
      currentCliId: mocks.currentCliId,
      visibleCliIds: mocks.visibleCliIds,
      cliOptions: ref(cliOptions),
      cliPathConfigs: ref({ claude: null, codex: null, gemini: null, workbuddy: null, dsh: null }),
      skipPermissions: ref(true),
      setSkipPermissions: mocks.setSkipPermissions,
      setCliDataDirOverride: mocks.setCliDataDirOverride,
      registerContextMenu: mocks.registerContextMenu,
      unregisterContextMenu: mocks.unregisterContextMenu,
      refresh: mocks.refresh,
      searchIndexProgress: ref(null),
    }),
  };
});

const { mount, flushPromises } = await import("@vue/test-utils");
const { t } = await import("../i18n");
const ArchivedSessionsSettings = (await import("./ArchivedSessionsSettings.vue")).default;
const IntegrationSettings = (await import("./IntegrationSettings.vue")).default;

beforeEach(() => {
  storage.clear();
  mocks.invoke.mockReset();
  mocks.setCliDataDirOverride.mockClear();
  mocks.currentCliId!.value = "claude";
  mocks.visibleCliIds!.value = ["claude"];
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_archived_sessions") return [];
    if (command === "get_archive_retention_days") return 60;
    if (command === "is_context_menu_registered") return false;
    return undefined;
  });
});

describe("ArchivedSessionsSettings 受控 cliId", () => {
  it("按 cliId prop 加载并随 prop 变化重载", async () => {
    const wrapper = mount(ArchivedSessionsSettings, {
      props: { cliId: "codex" },
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("list_archived_sessions", { cliId: "codex" });

    mocks.invoke.mockClear();
    await wrapper.setProps({ cliId: "claude" });
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("list_archived_sessions", { cliId: "claude" });
    // 内部选择器已删除
    expect(wrapper.find('[data-testid="archive-cli-select"]').exists()).toBe(false);
  });

  it("标题行展示计数与扫盘按钮", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_archived_sessions") {
        return [{
          sessionPath: "/archived/s.jsonl",
          sessionId: "s",
          projectPath: "/p",
          displayName: "归档对话",
          firstUserMessage: null,
          archivedAt: "2026-08-27T08:00:00Z",
          pinned: false,
          sourceExists: true,
        }];
      }
      return undefined;
    });
    const wrapper = mount(ArchivedSessionsSettings, {
      props: { cliId: "codex" },
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    // 此处不需要「不得渲染旧工具条说明」的否定断言：那句话的原文
    // "受永久保留保护的快照及历史归档会话包" 在全部 git 历史里只存在于断言自身，
    // 四份语言包也没有任何 key 承载它，组件没有能渲染出它的代码路径。
    // 唯一能让它重新出现的做法是在本组件里硬编码中文，而闸门规则 1 已覆盖该文件。
    // 语义上最接近的 settings.archivedSessions.hint 恰恰是当前就该渲染的文案，
    // 拿它做否定断言会恒为假。标题行的现状由下方两条正向断言守住。
    expect(wrapper.text()).toContain(t("settings.archivedSessions.listTitle", { count: 1 }));
    expect(wrapper.find(".archive-list-header button").attributes("title")).toBe(
      t("settings.archivedSessions.scanTitle"),
    );
  });

  it("opens sessions with the controlled cliId identity", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_archived_sessions") {
        return [{
          sessionPath: "/archived/s.jsonl",
          sessionId: "s",
          projectPath: "/p",
          displayName: "归档对话",
          firstUserMessage: null,
          archivedAt: "2026-08-27T08:00:00Z",
          pinned: false,
          sourceExists: true,
        }];
      }
      return undefined;
    });
    const wrapper = mount(ArchivedSessionsSettings, {
      props: { cliId: "codex" },
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    await wrapper.get(".entry-row").trigger("click");

    expect(wrapper.emitted("openSession")).toEqual([
      [{ cliId: "codex", filePath: "/archived/s.jsonl" }],
    ]);
  });

  it("行内操作：源文件存在时仅展示置顶与删除，源文件丢失时展示恢复写回磁盘", async () => {
    // 场景 1：源文件存在（已快照）
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_archived_sessions") {
        return [{
          sessionPath: "/archived/s.jsonl",
          sessionId: "s",
          projectPath: "/p",
          displayName: "归档对话",
          firstUserMessage: null,
          archivedAt: "2026-08-27T08:00:00Z",
          pinned: false,
          sourceExists: true,
        }];
      }
      return undefined;
    });
    const wrapper = mount(ArchivedSessionsSettings, {
      props: { cliId: "codex" },
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    let buttons = wrapper.findAll(".entry-actions .row-icon-btn");
    expect(buttons).toHaveLength(2);
    expect(buttons[0].attributes("title")).toBe(t("settings.archivedSessions.pinOn"));
    expect(buttons[1].attributes("title")).toBe(t("settings.archivedSessions.deleteSnapshotTitle"));

    // 场景 2：源文件已被 CLI 清理（已归档）
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_archived_sessions") {
        return [{
          sessionPath: "/archived/s.jsonl",
          sessionId: "s",
          projectPath: "/p",
          displayName: "已清理对话",
          firstUserMessage: null,
          archivedAt: "2026-08-27T08:00:00Z",
          pinned: false,
          sourceExists: false,
        }];
      }
      return undefined;
    });
    await wrapper.setProps({ cliId: "claude" });
    await flushPromises();

    buttons = wrapper.findAll(".entry-actions .row-icon-btn");
    expect(buttons).toHaveLength(3);
    expect(buttons[0].attributes("title")).toBe(t("settings.archivedSessions.pinOn"));
    expect(buttons[1].attributes("title")).toBe(t("settings.archivedSessions.restoreTitle"));
    expect(buttons[2].attributes("title")).toBe(t("settings.archivedSessions.deleteArchiveTitle"));
  });
});

const ArchiveRetentionSettings = (await import("./ArchiveRetentionSettings.vue")).default;
const CacheMaintenanceSettings = (await import("./CacheMaintenanceSettings.vue")).default;

describe("ArchiveRetentionSettings 无极进度条与快照保留", () => {
  it("加载已保存天数并渲染滑块与标尺，点击里程碑或滑动触发保存", async () => {
    mocks.invoke.mockImplementation(async (command: string, args?: any) => {
      if (command === "get_archive_retention_days") return 60;
      if (command === "set_archive_retention_days") return undefined;
      return undefined;
    });
    const wrapper = mount(ArchiveRetentionSettings, {
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    expect(wrapper.text()).toContain(t("settings.archive.title"));
    expect(wrapper.find(".retention-badge").text()).toBe(
      t("settings.archive.daysCount", { count: 60 }),
    );

    // 点击 1 年里程碑按钮
    const btn1Year = wrapper.findAll(".milestone-btn").find((b) => b.text() === t("settings.archive.milestone1y"));
    expect(btn1Year).toBeDefined();
    await btn1Year!.trigger("click");
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("set_archive_retention_days", { days: 365 });
    expect(wrapper.find(".retention-badge").text()).toBe(
      t("settings.archive.yearsDays", { years: "1", days: 365 }),
    );

    // 点击 永久保留 里程碑按钮
    const btnForever = wrapper.findAll(".milestone-btn").find((b) => b.text() === t("settings.archive.milestoneForever"));
    expect(btnForever).toBeDefined();
    await btnForever!.trigger("click");
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("set_archive_retention_days", { days: -1 });
    expect(wrapper.find(".retention-badge").text()).toBe(t("settings.archive.milestoneForever"));
  });
});

describe("CacheMaintenanceSettings 临时配置维护", () => {
  it("点击立即清理按钮触发 clean_temp_configs", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "clean_temp_configs") return 3;
      return undefined;
    });
    const wrapper = mount(CacheMaintenanceSettings, {
      global: { stubs: { SvgIcon: true } },
    });
    await flushPromises();

    expect(wrapper.text()).toContain(t("settings.cache.tempConfigTitle"));
    const cleanBtn = wrapper.find("button.btn-action");
    expect(cleanBtn.exists()).toBe(true);
    await cleanBtn.trigger("click");
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("clean_temp_configs");
  });
});

describe("IntegrationSettings feature CLI", () => {
  it("shows the selected CLI's permission copy and context-menu capability", async () => {
    const wrapper = mount(IntegrationSettings, {
      global: { stubs: { SvgIcon: true, ToggleSwitch: true } },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("Skip permission checks");

    await wrapper.findComponent('[data-testid="integration-cli-select"]').vm.$emit("update:modelValue", "codex");
    await nextTick();

    expect(wrapper.text()).toContain("Bypass approvals and sandbox");
    expect(wrapper.text()).toContain(t("settings.integration.contextMenuUnsupported"));
    expect(mocks.currentCliId!.value).toBe("claude");
  });

  it("registers the context menu through the locally selected CLI", async () => {
    const wrapper = mount(IntegrationSettings, {
      global: {
        stubs: {
          SvgIcon: true,
          ToggleSwitch: {
            props: ["modelValue"],
            emits: ["update:modelValue"],
            template: `<button class="toggle-stub" @click="$emit('update:modelValue', !modelValue)"></button>`,
          },
        },
      },
    });
    await flushPromises();

    const toggles = wrapper.findAll(".toggle-stub");
    // 第二个开关是系统右键菜单（仅 claude 支持）
    await toggles[1].trigger("click");
    await flushPromises();

    expect(mocks.registerContextMenu).toHaveBeenCalledWith("claude");
  });
});

describe("DataIndexSettings 页面结构", () => {
  it("CLI 索引与归档合为单卡，子组件共享同一 CLI 上下文", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_archived_sessions") return [];
      if (command === "get_archive_retention_days") return 60;
      if (command === "get_index_stats") {
        return { sessionCount: 5, dbSizeBytes: 100, searchDocCount: 5, searchIndexBytes: 1000, lastUpdatedMs: null };
      }
      return undefined;
    });
    const DataIndexSettings = (await import("./DataIndexSettings.vue")).default;
    const wrapper = mount(DataIndexSettings, {
      props: { initialCliId: "claude" },
      global: {
        stubs: {
          SvgIcon: true,
          ElegantSelect: { props: ["modelValue", "options"], template: "<select></select>" },
          ToggleSwitch: true,
          // 屏蔽文件夹卡与本测试无关，stub 掉避免其 composable 的 invoke 依赖
          BlockedFoldersSettings: true,
        },
      },
    });
    await flushPromises();

    // 归档列表随 initialCliId 加载
    expect(mocks.invoke).toHaveBeenCalledWith("list_archived_sessions", { cliId: "claude" });

    // 切换 CLI 选择器 → 索引统计与归档列表都重载
    mocks.invoke.mockClear();
    await wrapper
      .findComponent('[data-testid="index-cli-select"]')
      .vm.$emit("update:modelValue", "codex");
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("get_index_stats", { cliId: "codex" });
    expect(mocks.invoke).toHaveBeenCalledWith("list_archived_sessions", { cliId: "codex" });
  });
});
