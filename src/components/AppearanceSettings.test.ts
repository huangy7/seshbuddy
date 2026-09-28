import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  changeLocale: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

// useTheme 在模块求值时订阅跨窗口主题广播，jsdom 里没有 Tauri IPC 可用
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: vi.fn(async () => () => {}),
}));

// 只替换 changeLocale：t 与 currentLocale 用真实实现，
// 这样断言的是组件真的绑在响应式的 locale 上，而不是绑在某个静态快照上。
vi.mock("../i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../i18n")>();
  return {
    ...actual,
    changeLocale: async (locale: string) => {
      mocks.changeLocale(locale);
      actual.currentLocale.value = locale as never;
    },
  };
});

// useTheme 在模块求值时读 localStorage，须在引入组件前替换掉运行环境里的实现
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});

const { t, setLocale } = await import("../i18n");
const AppearanceSettings = (await import("./AppearanceSettings.vue")).default;

function languageRow(wrapper: VueWrapper) {
  const row = wrapper
    .findAll(".settings-row")
    .find((r) => r.find(".row-title").text() === t("common.language.title"));
  if (!row) throw new Error(`未找到标题为 ${t("common.language.title")} 的设置行`);
  return row;
}

describe("AppearanceSettings.vue 界面语言选择器", () => {
  beforeEach(() => {
    setLocale("en");
    mocks.changeLocale.mockReset();
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(true);
  });

  it("渲染四个语言选项，语言名用本族语书写", () => {
    // hint 断言放在非基准语言下：在 en 下断言，组件里硬编码一份英文 hint 同样通过。
    setLocale("zh");
    const wrapper = mount(AppearanceSettings);

    const labels = languageRow(wrapper)
      .findAll("button")
      .map((btn) => btn.text());

    expect(labels).toEqual(["English", "简体中文", "日本語", "Deutsch"]);
    expect(languageRow(wrapper).find(".row-hint").text()).toBe(t("common.language.hint"));
  });

  it("选中项跟随 currentLocale", async () => {
    setLocale("ja");
    const wrapper = mount(AppearanceSettings);

    const active = languageRow(wrapper).findAll("button").find((btn) => btn.attributes("aria-pressed") === "true");

    expect(active?.text()).toBe("日本語");
  });

  it("点击 ja 以 'ja' 调用 changeLocale，界面随之切换", async () => {
    const wrapper = mount(AppearanceSettings);

    const ja = languageRow(wrapper)
      .findAll("button")
      .find((btn) => btn.text() === "日本語");
    expect(ja).toBeDefined();

    await ja!.trigger("click");

    expect(mocks.changeLocale).toHaveBeenCalledTimes(1);
    expect(mocks.changeLocale).toHaveBeenCalledWith("ja");
    expect(languageRow(wrapper).find("button[aria-pressed='true']").text()).toBe("日本語");
  });
});
