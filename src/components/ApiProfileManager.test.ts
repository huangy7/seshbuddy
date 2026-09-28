import { describe, it, expect, vi, beforeEach } from "vitest";
import { shallowMount, flushPromises } from "@vue/test-utils";
import { ref } from "vue";
import ApiProfileManager from "./ApiProfileManager.vue";
import ProfileImportConflictDialog from "./api-profile/ProfileImportConflictDialog.vue";
import { t } from "../i18n";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  ask: vi.fn(),
  message: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
  copyPromiseToClipboard: vi.fn(),
  readText: vi.fn(),
  addProxyStatusChangedListener: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: mocks.ask,
  message: mocks.message,
  open: mocks.open,
  save: mocks.save,
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: mocks.readText,
}));
vi.mock("../utils/clipboard", () => ({
  copyPromiseToClipboard: mocks.copyPromiseToClipboard,
}));
vi.mock("../composables/useProxy", () => ({
  addProxyStatusChangedListener: mocks.addProxyStatusChangedListener,
}));

const mockCliOptions: CliOption[] = (Object.keys(CLI_DEFINITIONS) as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: true,
  hasBinary: true,
}));

vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({
    cliOptions: ref(mockCliOptions),
  }),
}));

let localStore: Record<string, string> = {};
vi.stubGlobal("localStorage", {
  getItem: (key: string) => localStore[key] ?? null,
  setItem: (key: string, val: string) => { localStore[key] = val; },
  removeItem: (key: string) => { delete localStore[key]; },
  clear: () => { localStore = {}; },
});

/** mount 期各命令的默认响应 */
function defaultResponse(cmd: string): unknown {
  switch (cmd) {
    case "list_profile_tabs":
    case "list_profiles":
    case "get_scope_bindings":
    case "get_scope_dir_status":
      return [];
    case "read_scope_settings":
      return "{}";
    case "proxy_status":
      return { enabled: false, running: false, port: 18080 };
    default:
      return null;
  }
}

function mockInvokeWith(overrides: Record<string, unknown>) {
  mocks.invoke.mockImplementation(async (cmd: string) => {
    if (cmd in overrides) {
      const value = overrides[cmd];
      if (value instanceof Error) throw value;
      return value;
    }
    return defaultResponse(cmd);
  });
}

async function mountManager() {
  const wrapper = shallowMount(ApiProfileManager);
  await flushPromises();
  return wrapper;
}

type ManagerWrapper = Awaited<ReturnType<typeof mountManager>>;

async function openMenu(wrapper: ManagerWrapper, index: 0 | 1) {
  const btn = wrapper.findAll(".header-actions .btn-header-tool")[index];
  await btn.trigger("click");
}

async function clickMenuItem(wrapper: ManagerWrapper, label: string) {
  const item = wrapper.findAll(".backup-menu button").find((b) => b.text() === label);
  expect(item, `菜单项 ${label} 应存在`).toBeDefined();
  await item!.trigger("click");
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.invoke.mockImplementation(async (cmd: string) => defaultResponse(cmd));
  mocks.listen.mockResolvedValue(() => {});
  mocks.addProxyStatusChangedListener.mockReturnValue(() => {});
  mocks.copyPromiseToClipboard.mockImplementation((p: Promise<string>) =>
    p.then(() => true),
  );
});

describe("ApiProfileManager 备份菜单", () => {
  it("导入/导出按钮展开下拉菜单，各含文件与剪贴板两个通道", async () => {
    const wrapper = await mountManager();

    await openMenu(wrapper, 0);
    let items = wrapper.findAll(".backup-menu button").map((b) => b.text());
    expect(items).toEqual([
      t("api-profile.page.importFromFile"),
      t("api-profile.page.importFromClipboard"),
    ]);

    await openMenu(wrapper, 1);
    items = wrapper.findAll(".backup-menu button").map((b) => b.text());
    expect(items).toEqual([
      t("api-profile.page.exportToFile"),
      t("api-profile.page.copyToClipboard"),
    ]);
  });

  it("点击组件外部后菜单关闭", async () => {
    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    expect(wrapper.find(".backup-menu").exists()).toBe(true);

    document.body.click();
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".backup-menu").exists()).toBe(false);
  });
});

describe("ApiProfileManager 导出到剪贴板", () => {
  it("确认警告后导出全部配置并写入剪贴板（不传 savePath）", async () => {
    mocks.ask.mockResolvedValue(true);
    const backupData = { version: 1, timestamp: "t", profiles: [], tabs: [] };
    mockInvokeWith({ export_profiles_backup: backupData });

    const wrapper = await mountManager();
    await openMenu(wrapper, 1);
    await clickMenuItem(wrapper, t("api-profile.page.copyToClipboard"));
    await flushPromises();

    expect(mocks.ask).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith("export_profiles_backup", {});
    expect(mocks.copyPromiseToClipboard).toHaveBeenCalledOnce();
    const passed = mocks.copyPromiseToClipboard.mock.calls[0][0] as Promise<string>;
    await expect(passed).resolves.toBe(JSON.stringify(backupData, null, 2));
  });

  it("取消警告后不发起导出", async () => {
    mocks.ask.mockResolvedValue(false);
    const wrapper = await mountManager();
    await openMenu(wrapper, 1);
    await clickMenuItem(wrapper, t("api-profile.page.copyToClipboard"));
    await flushPromises();

    expect(mocks.invoke).not.toHaveBeenCalledWith("export_profiles_backup", expect.anything());
    expect(mocks.copyPromiseToClipboard).not.toHaveBeenCalled();
  });
});

describe("ApiProfileManager 从剪贴板导入", () => {
  it("剪贴板为空时提示且不调解析命令", async () => {
    mocks.readText.mockResolvedValue("   ");
    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, t("api-profile.page.importFromClipboard"));
    await flushPromises();

    expect(mocks.message).toHaveBeenCalledWith(
      expect.stringContaining(t("api-profile.importClipboard.empty")),
      expect.objectContaining({ kind: "warning" }),
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("parse_profiles_backup", expect.anything());
  });

  it("剪贴板内容非法时提示导入失败", async () => {
    mocks.readText.mockResolvedValue("not json");
    mockInvokeWith({ parse_profiles_backup: new Error("解析备份内容失败: ...") });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, t("api-profile.page.importFromClipboard"));
    await flushPromises();

    expect(mocks.message).toHaveBeenCalledWith(
      expect.stringContaining(t("api-profile.importClipboard.invalid", { error: "" })),
      expect.objectContaining({ kind: "error" }),
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("import_profiles_backup", expect.anything());
  });

  it("合法备份且无冲突时直接执行导入", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [
        { cli_id: "claude", scope: "global", name: "test", content: "{}", is_active: false },
      ],
      tabs: [],
    };
    mockInvokeWith({ parse_profiles_backup: backupData });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, t("api-profile.page.importFromClipboard"));
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("parse_profiles_backup", {
      content: '{"version":1}',
    });
    expect(mocks.invoke).toHaveBeenCalledWith(
      "import_profiles_backup",
      expect.objectContaining({
        items: [expect.objectContaining({ name: "test", action: "overwrite" })],
      }),
    );
  });

  it("备份中的作用域绑定随导入透传给后端", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [
        { cli_id: "claude", scope: "global", name: "kimi", content: "{}", is_active: false },
      ],
      tabs: [],
      scope_bindings: [
        { cli_id: "claude", scope: "OYxV4yFhv6", profile_name: "kimi" },
      ],
    };
    mockInvokeWith({ parse_profiles_backup: backupData });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, t("api-profile.page.importFromClipboard"));
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith(
      "import_profiles_backup",
      expect.objectContaining({
        scopeBindings: [
          { cli_id: "claude", scope: "OYxV4yFhv6", profile_name: "kimi" },
        ],
      }),
    );
  });

  it("作用域同名不同 id 触发冲突，跳过的作用域不传给后端", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [{ cli_id: "claude", scope: "global", name: "a", content: "{}", is_active: false }],
      tabs: [{ id: "t1", cli_id: "claude", name: "scope-x", dirs: ["/d"] }],
      scope_bindings: [{ cli_id: "claude", scope: "t1", profile_name: "a" }],
    };
    mocks.invoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "parse_profiles_backup") return backupData;
      // 本地有一个同名的 scope-x，但 id 不同（t-local）
      if (cmd === "list_profile_tabs") {
        return [{ id: "t-local", cli_id: "claude", name: "scope-x", dirs: [] }];
      }
      return defaultResponse(cmd);
    });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, t("api-profile.page.importFromClipboard"));
    await flushPromises();

    // 同名不同 id → 触发对话框，作用域状态为重名冲突、默认跳过
    const dialog = wrapper.findComponent(ProfileImportConflictDialog);
    expect(dialog.exists()).toBe(true);
    const scopes = dialog.props("scopes") as Array<{ id: string; name: string; status: string; action: string }>;
    expect(scopes[0].status).toBe("nameConflict");
    expect(scopes[0].action).toBe("skip");

    // 模拟用户在对话框里保持默认（跳过该作用域），确认导入
    await dialog.vm.$emit("confirm", [], scopes);
    await flushPromises();
    const call = mocks.invoke.mock.calls.find((c) => c[0] === "import_profiles_backup");
    expect(call).toBeDefined();
    // 跳过的作用域不应进入 tabs 载荷
    expect((call![1] as any).tabs).toEqual([]);
  });

  it("冲突检测按备份中各 CLI 的本地配置逐一比对（多 CLI 备份）", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [
        { cli_id: "claude", scope: "global", name: "claude", content: "{}", is_active: false },
        { cli_id: "codex", scope: "global", name: "default", content: "{}", is_active: false },
      ],
      tabs: [],
    };
    mocks.invoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "parse_profiles_backup") return backupData;
      // 本地 claude 有 "claude"，codex 有 "default" —— 两条都应判为冲突
      if (cmd === "list_profiles") return args?.cliId === "codex" ? ["default"] : ["claude"];
      return defaultResponse(cmd);
    });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, t("api-profile.page.importFromClipboard"));
    await flushPromises();

    const dialog = wrapper.findComponent(ProfileImportConflictDialog);
    expect(dialog.exists()).toBe(true);
    const candidates = dialog.props("candidates") as Array<{ name: string; isConflict: boolean }>;
    expect(candidates).toHaveLength(2);
    expect(candidates.find((c) => c.name === "claude")?.isConflict).toBe(true);
    expect(candidates.find((c) => c.name === "default")?.isConflict).toBe(true);
  });
});

describe("ApiProfileManager 配置文件路径与预览", () => {
  it("默认 Claude CLI 时配置文件提示为 settings.json", async () => {
    const wrapper = await mountManager();
    const listComponent = wrapper.findComponent({ name: "ApiProfileList" });
    expect(listComponent.exists()).toBe(true);
    expect(listComponent.props("currentSettingsPathHint")).toContain("settings.json");
    expect(listComponent.props("currentCliId")).toBe("claude");
  });
});

describe("ApiProfileManager 显示名称的自动/手填", () => {
  const BASE_URL = "https://gateway.example.com/v1";
  const CACHE_KEY = `seshbuddy-model-list-cache:${BASE_URL}`;

  /** 配置里已经存着一个等于网关 display 的名字（旧版本自动写进去的） */
  const profileContent = {
    env: {
      ANTHROPIC_BASE_URL: BASE_URL,
      ANTHROPIC_DEFAULT_SONNET_MODEL: "claude-sonnet-4-5",
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "Claude Sonnet 4.5",
    },
  };

  function seedModelCache() {
    localStore[CACHE_KEY] = JSON.stringify({
      base_url: BASE_URL,
      models: [
        { id: "claude-sonnet-4-5", display: "Claude Sonnet 4.5" },
        { id: "kimi-k2.6", display: "Kimi K2.6" },
      ],
      fetched_at: "2026-09-29T00:00:00Z",
    });
  }

  async function openSonnetEditor() {
    mockInvokeWith({
      list_profiles: ["p1"],
      read_profile: JSON.stringify(profileContent),
    });
    const wrapper = await mountManager();
    wrapper.findComponent({ name: "ApiProfileList" }).vm.$emit("openEditor", "p1");
    await flushPromises();
    return wrapper;
  }

  async function openSonnetProfile() {
    const wrapper = await openSonnetEditor();
    return wrapper.findComponent({ name: "ApiProfileModelOverridesSection" });
  }

  /** 切到 JSON 视图，用改过的 env 提交一次编辑，再切回表单 */
  async function editInJsonView(wrapper: ManagerWrapper, env: Record<string, string>) {
    const header = wrapper.findComponent({ name: "ApiProfileEditorHeader" });
    header.vm.$emit("switch-view", "json");
    await flushPromises();
    wrapper
      .findComponent({ name: "ApiProfileJsonEditor" })
      .vm.$emit("update-json", JSON.stringify({ env: { ...profileContent.env, ...env } }));
    await flushPromises();
    header.vm.$emit("switch-view", "form");
    await flushPromises();
    return wrapper.findComponent({ name: "ApiProfileModelOverridesSection" });
  }

  beforeEach(() => {
    seedModelCache();
  });

  it("旧配置里等于 display 的名字，切模型时跟随新模型更新", async () => {
    const section = await openSonnetProfile();
    expect(section.props("sonnetModelName")).toBe("Claude Sonnet 4.5");

    section.vm.$emit("select-model", "sonnet", "kimi-k2.6");
    await flushPromises();

    expect(section.props("sonnetModelName")).toBe("Kimi K2.6");
    expect(section.props("typedNames")).toEqual({ haiku: false, sonnet: false, opus: false });
  });

  it("手填过的名字，切模型时保留", async () => {
    const section = await openSonnetProfile();

    section.vm.$emit("update-model-name", "sonnet", "我的主力模型");
    await flushPromises();
    section.vm.$emit("select-model", "sonnet", "kimi-k2.6");
    await flushPromises();

    expect(section.props("sonnetModelName")).toBe("我的主力模型");
    expect(section.props("typedNames")).toEqual({ haiku: false, sonnet: true, opus: false });
  });

  it("清空过的名字，切模型时保持为空", async () => {
    const section = await openSonnetProfile();

    section.vm.$emit("update-model-name", "sonnet", "");
    await flushPromises();
    section.vm.$emit("select-model", "sonnet", "kimi-k2.6");
    await flushPromises();

    expect(section.props("sonnetModelName")).toBe("");
  });

  it("点「还原自动」后该行回到跟随模型", async () => {
    const section = await openSonnetProfile();

    section.vm.$emit("update-model-name", "sonnet", "我的主力模型");
    await flushPromises();
    expect(section.props("sonnetModelName")).toBe("我的主力模型");

    section.vm.$emit("restore-auto-name", "sonnet");
    await flushPromises();
    expect(section.props("sonnetModelName")).toBe("Claude Sonnet 4.5");
    expect(section.props("typedNames")).toEqual({ haiku: false, sonnet: false, opus: false });

    section.vm.$emit("select-model", "sonnet", "kimi-k2.6");
    await flushPromises();
    expect(section.props("sonnetModelName")).toBe("Kimi K2.6");
  });

  it("点「还原自动」也能把清空过的行拉回跟随", async () => {
    const section = await openSonnetProfile();

    section.vm.$emit("update-model-name", "sonnet", "");
    await flushPromises();

    section.vm.$emit("restore-auto-name", "sonnet");
    await flushPromises();

    expect(section.props("sonnetModelName")).toBe("Claude Sonnet 4.5");
  });

  it("JSON 视图里同时改了模型和名字，名字按手填保留", async () => {
    const wrapper = await openSonnetEditor();
    const section = await editInJsonView(wrapper, {
      ANTHROPIC_DEFAULT_SONNET_MODEL: "kimi-k2.6",
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "JSON 里手写的名字",
    });

    expect(section.props("sonnetModel")).toBe("kimi-k2.6");
    expect(section.props("sonnetModelName")).toBe("JSON 里手写的名字");
    expect(section.props("typedNames")).toEqual({ haiku: false, sonnet: true, opus: false });
  });

  it("JSON 视图里只改模型，回到表单后名字跟随新模型", async () => {
    const wrapper = await openSonnetEditor();
    const section = await editInJsonView(wrapper, { ANTHROPIC_DEFAULT_SONNET_MODEL: "kimi-k2.6" });

    expect(section.props("sonnetModelName")).toBe("Kimi K2.6");
    expect(section.props("typedNames")).toEqual({ haiku: false, sonnet: false, opus: false });
  });
});
