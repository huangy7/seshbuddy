import { computed } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { currentLocale, normalizeLocale, setLocale, t } from "./index";
import enCommon from "../locales/en/common.json";
import zhCommon from "../locales/zh/common.json";

// vi.mock 的工厂会被提升到文件顶部，引用普通顶层变量会撞上 TDZ，
// 故用 vi.hoisted 把 mock 实例一并提到提升区。
const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const eventMock = vi.hoisted(() => ({ emit: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => eventMock);

describe("i18n 实例", () => {
  it("默认 locale 是 en", () => {
    expect(currentLocale.value).toBe("en");
  });

  it("setLocale 切换后 t 读新语言包", () => {
    // 断言 t 的取值来自当前语言包，而非钉住具体措辞：
    // 措辞调整只该改语言包，不该让本用例变红。
    setLocale("en");
    expect(t("common.select.placeholder")).toBe(enCommon.select.placeholder);

    setLocale("zh");
    expect(currentLocale.value).toBe("zh");
    expect(t("common.select.placeholder")).toBe(zhCommon.select.placeholder);
    setLocale("en");
  });

  it("缺失 key 经回退链返回 key 本身而非抛异常", () => {
    // 在非基准语言下查一个四份语言包都没有的 key，回退链（zh → en）才会真的走一遍。
    // 「回退到 en 的**值**」这一分支构造不出来：闸门规则 3 强制 en 的每个 key 在其它语言里都有条目。
    setLocale("zh");
    expect(t("__nonexistent__.key")).toBe("__nonexistent__.key");
    setLocale("en");
  });

  it("模块级 t 在渲染外追踪 locale 变化", () => {
    // 迁移后的文案多由 composables 里的 computed 消费，若 t 不读响应式的 locale，
    // 切换语言时这些 computed 不会重算，界面语言就停在旧语言。
    // 这里用求值次数判断依赖是否被追踪——比比较返回值更严格：
    // 即使两种语言恰好返回同一字符串，依赖断了也算失败。
    setLocale("en");
    let runs = 0;
    const localized = computed(() => {
      runs += 1;
      return t("__nonexistent__.key");
    });

    expect(localized.value).toBe("__nonexistent__.key");
    expect(runs).toBe(1);

    setLocale("zh");
    expect(localized.value).toBe("__nonexistent__.key");
    expect(runs).toBe(2);

    setLocale("en");
  });

  it("computed 会随 locale 变化重算", () => {
    setLocale("en");
    const c = computed(() => currentLocale.value);
    expect(c.value).toBe("en");
    setLocale("zh");
    expect(c.value).toBe("zh");
    setLocale("en");
  });

  it("zh-TW 归一化到 en 而非 zh", () => {
    // 繁体界面尚未提供，回退基准语言比端上一份简体界面更诚实
    expect(normalizeLocale("zh-TW")).toBe("en");
  });
});

describe("initLocale", () => {
  // test-setup 会先 import 本模块，模块缓存里已绑定真实的 invoke，
  // 此处的 vi.mock 对缓存实例无效；resetModules 让每个用例拿到重新求值的实例。
  beforeEach(() => {
    vi.resetModules();
    invokeMock.mockReset();
  });

  it("从后端取回 locale 并应用", async () => {
    invokeMock.mockResolvedValue("ja");
    const { initLocale, currentLocale } = await import("./index");
    await initLocale();
    expect(currentLocale.value).toBe("ja");
  });

  it("后端失败时保持基准语言，不抛异常", async () => {
    invokeMock.mockRejectedValue(new Error("boom"));
    const { initLocale, currentLocale } = await import("./index");
    await expect(initLocale()).resolves.toBeUndefined();
    expect(currentLocale.value).toBe("en");
  });
});

describe("changeLocale", () => {
  beforeEach(() => {
    vi.resetModules();
    invokeMock.mockReset();
    eventMock.emit.mockReset();
    eventMock.emit.mockResolvedValue(undefined);
    eventMock.listen.mockResolvedValue(() => {});
  });

  it("语言落定后广播给其它窗口", async () => {
    // 助手窗口是独立 webview，只有收到这条广播才会换语言
    invokeMock.mockResolvedValue("ja");
    const { changeLocale } = await import("./index");
    await changeLocale("ja");
    expect(eventMock.emit).toHaveBeenCalledWith("app-locale-changed", "ja");
  });

  it("后端保存失败时仍广播本地已应用的语言", async () => {
    // 主窗口此刻已经切到 de，广播要跟着走，否则两个窗口语言不一致
    invokeMock.mockRejectedValue(new Error("boom"));
    const { changeLocale, currentLocale } = await import("./index");
    await changeLocale("de");
    expect(currentLocale.value).toBe("de");
    expect(eventMock.emit).toHaveBeenCalledWith("app-locale-changed", "de");
  });

  it("向引导页报告语言是否成功保存", async () => {
    const { changeLocale } = await import("./index");
    invokeMock.mockResolvedValueOnce("zh").mockRejectedValueOnce(new Error("boom"));
    expect(await changeLocale("zh")).toBe(true);
    expect(await changeLocale("de")).toBe(false);
  });
});
