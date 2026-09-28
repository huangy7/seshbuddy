import { beforeEach, describe, expect, it, vi } from "vitest";

// 通道模块在 import 时就绑定 emit / listen，故 mock 必须先于 import 登记。
const mocks = vi.hoisted(() => ({ emit: vi.fn(), listen: vi.fn() }));

vi.mock("@tauri-apps/api/event", () => ({ emit: mocks.emit, listen: mocks.listen }));

// 运行环境的 localStorage 是个空对象（被 Node 的同名全局遮住），
// 与仓库既有的做法一致：测试里换成 Map 实现。
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});

/**
 * localeChannel 被 test-setup（经 `i18n/index.ts`）先行加载，模块缓存里已绑定真实的
 * event 模块，此处的 vi.mock 对缓存实例无效；resetModules 让每个用例拿到重新求值的实例。
 */
async function loadChannel() {
  vi.resetModules();
  return await import("./localeChannel");
}

/** listen 的回调形态（载荷在 event.payload 上）。 */
type BroadcastHandler = (event: { payload: unknown }) => void;

describe("跨窗口语言通道", () => {
  beforeEach(() => {
    storage.clear();
    mocks.emit.mockReset();
    mocks.listen.mockReset();
    mocks.emit.mockResolvedValue(undefined);
    mocks.listen.mockResolvedValue(() => {});
  });

  it("broadcastLocale 广播事件并写 localStorage 兜底", async () => {
    const { broadcastLocale, LOCALE_CHANGED_EVENT, LOCALE_STORAGE_KEY } = await loadChannel();

    broadcastLocale("zh");

    expect(mocks.emit).toHaveBeenCalledWith(LOCALE_CHANGED_EVENT, "zh");
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh");
  });

  it("emit 不可用时广播不抛异常，兜底仍写", async () => {
    // 无 Tauri IPC 的环境（浏览器预览）里 emit 会 reject；
    // 广播失败只该影响别的窗口，不该把语言切换本身弄崩。
    const { broadcastLocale, LOCALE_STORAGE_KEY } = await loadChannel();
    mocks.emit.mockImplementation(() => Promise.reject(new Error("no ipc")));

    expect(() => broadcastLocale("ja")).not.toThrow();
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("ja");
  });

  it("onLocaleBroadcast 把广播里的合法语言交给 apply，非法载荷忽略", async () => {
    const { onLocaleBroadcast, LOCALE_CHANGED_EVENT } = await loadChannel();
    let handler: BroadcastHandler | undefined;
    mocks.listen.mockImplementation((_event: string, cb: BroadcastHandler) => {
      handler = cb;
      return Promise.resolve(() => {});
    });

    const apply = vi.fn();
    onLocaleBroadcast(apply);

    expect(mocks.listen).toHaveBeenCalledWith(LOCALE_CHANGED_EVENT, expect.any(Function));

    handler!({ payload: "ja" });
    expect(apply).toHaveBeenCalledWith("ja");

    handler!({ payload: "fr" });
    handler!({ payload: null });
    expect(apply).toHaveBeenCalledTimes(1);
  });

  it("storage 事件是兜底通道，其它键与非法值忽略，退订后停止", async () => {
    const { onLocaleBroadcast, LOCALE_STORAGE_KEY } = await loadChannel();
    const tauriUnlisten = vi.fn();
    mocks.listen.mockResolvedValue(tauriUnlisten);

    const apply = vi.fn();
    const off = onLocaleBroadcast(apply);
    await Promise.resolve();

    window.dispatchEvent(
      new StorageEvent("storage", { key: LOCALE_STORAGE_KEY, newValue: "de" }),
    );
    expect(apply).toHaveBeenCalledWith("de");

    window.dispatchEvent(
      new StorageEvent("storage", { key: "seshbuddy-theme", newValue: "zh" }),
    );
    window.dispatchEvent(
      new StorageEvent("storage", { key: LOCALE_STORAGE_KEY, newValue: "fr" }),
    );
    expect(apply).toHaveBeenCalledTimes(1);

    off();
    expect(tauriUnlisten).toHaveBeenCalledTimes(1);

    window.dispatchEvent(
      new StorageEvent("storage", { key: LOCALE_STORAGE_KEY, newValue: "zh" }),
    );
    expect(apply).toHaveBeenCalledTimes(1);
  });

  it("listen 落定前退订也要释放 Tauri 监听器", async () => {
    // 退订早于 listen 落定时 unlisten 还没拿到，不补一次就永久残留
    const { onLocaleBroadcast } = await loadChannel();
    const tauriUnlisten = vi.fn();
    mocks.listen.mockResolvedValue(tauriUnlisten);

    const off = onLocaleBroadcast(vi.fn());
    off();
    expect(tauriUnlisten).not.toHaveBeenCalled();

    await Promise.resolve();
    expect(tauriUnlisten).toHaveBeenCalledTimes(1);
  });
});
