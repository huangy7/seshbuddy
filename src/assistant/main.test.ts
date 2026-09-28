import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

// 助手外壳在模块顶层订阅语言广播，故 mock 必须先于 import 登记。
const mocks = vi.hoisted(() => ({
  emit: vi.fn(),
  listen: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ emit: mocks.emit, listen: mocks.listen }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
// 外壳只负责把组件挂起来，组件本身与指令不是本用例的对象
vi.mock("./AssistantApp.vue", () => ({
  default: { name: "AssistantApp", render: () => null },
}));
vi.mock("../directives/vMermaid", () => ({ vMermaid: { mounted: () => {} } }));

// 运行环境的 localStorage 是个空对象（被 Node 的同名全局遮住），
// useTheme 与 ModelPicker 一类的模块在加载/挂载时都要读它。
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});

/** listen 的回调形态（载荷在 event.payload 上）。 */
type BroadcastHandler = (event: { payload: unknown }) => void;

describe("助手窗口外壳", () => {
  beforeEach(() => {
    storage.clear();
    mocks.emit.mockReset();
    mocks.listen.mockReset();
    mocks.invoke.mockReset();
    mocks.emit.mockResolvedValue(undefined);
    mocks.listen.mockResolvedValue(() => {});
    mocks.invoke.mockResolvedValue("en");
    document.body.innerHTML = '<div id="app"></div>';
  });

  it("订阅语言广播，收到后本窗口的语言随之切换", async () => {
    // 外壳被 test-setup 之外的地方首次加载，但 i18n 与 localeChannel 在缓存里
    // 绑的是真实 event 模块；resetModules 让这份图整体重新求值，mock 才生效。
    vi.resetModules();
    const handlers = new Map<string, BroadcastHandler>();
    mocks.listen.mockImplementation((event: string, cb: BroadcastHandler) => {
      handlers.set(event, cb);
      return Promise.resolve(() => {});
    });

    const { currentLocale } = await import("../i18n");
    await import("./main");
    await flushPromises();

    const onLocaleChanged = handlers.get("app-locale-changed");
    expect(onLocaleChanged).toBeTypeOf("function");
    expect(currentLocale.value).toBe("en");

    onLocaleChanged!({ payload: "ja" });

    expect(currentLocale.value).toBe("ja");
  });
});
