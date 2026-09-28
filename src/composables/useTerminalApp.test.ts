import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  storage: new Map<string, string>(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

vi.stubGlobal("localStorage", {
  getItem: (key: string) => mocks.storage.get(key) ?? null,
  setItem: (key: string, value: string) => void mocks.storage.set(key, value),
  removeItem: (key: string) => void mocks.storage.delete(key),
  clear: () => mocks.storage.clear(),
  key: () => null,
  get length() {
    return mocks.storage.size;
  },
});

const STORAGE_KEY = "seshbuddy-terminal-app";

async function importModule() {
  // 模块级单例状态，每个用例需要全新模块
  vi.resetModules();
  return await import("./useTerminalApp");
}

describe("useTerminalApp", () => {
  beforeEach(() => {
    localStorage.clear();
    mocks.invoke.mockReset();
  });

  it("默认取探测列表第一项（推荐顺序由后端决定）", async () => {
    mocks.invoke.mockResolvedValue(["iterm2", "terminal"]);
    const { useTerminalApp } = await importModule();
    const { terminalApp, detectTerminalApps } = useTerminalApp();

    expect(terminalApp.value).toBeNull();
    await detectTerminalApps();
    expect(terminalApp.value).toBe("iterm2");
    expect(mocks.invoke).toHaveBeenCalledWith("detect_terminal_apps");
  });

  it("已保存且仍安装的选择优先于默认", async () => {
    localStorage.setItem(STORAGE_KEY, "terminal");
    mocks.invoke.mockResolvedValue(["iterm2", "terminal"]);
    const { useTerminalApp } = await importModule();
    const { terminalApp, detectTerminalApps } = useTerminalApp();

    await detectTerminalApps();
    expect(terminalApp.value).toBe("terminal");
  });

  it("保存值已失效（应用被卸载）时清除存储并回退默认", async () => {
    localStorage.setItem(STORAGE_KEY, "iterm2");
    mocks.invoke.mockResolvedValue(["terminal"]);
    const { useTerminalApp } = await importModule();
    const { terminalApp, detectTerminalApps } = useTerminalApp();

    await detectTerminalApps();
    expect(terminalApp.value).toBe("terminal");
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("terminalAppForLaunch 返回原始保存值，不依赖探测完成", async () => {
    localStorage.setItem(STORAGE_KEY, "iterm2");
    mocks.invoke.mockReturnValue(new Promise(() => {})); // 探测永不返回
    const { useTerminalApp } = await importModule();
    const { terminalAppForLaunch } = useTerminalApp();

    expect(terminalAppForLaunch.value).toBe("iterm2");
  });

  it("未保存时 terminalAppForLaunch 为 null，交由后端自动探测", async () => {
    const { useTerminalApp } = await importModule();
    const { terminalAppForLaunch } = useTerminalApp();

    expect(terminalAppForLaunch.value).toBeNull();
  });

  it("setTerminalApp 持久化并立即生效", async () => {
    mocks.invoke.mockResolvedValue(["iterm2", "terminal"]);
    const { useTerminalApp } = await importModule();
    const { terminalApp, terminalAppForLaunch, setTerminalApp, detectTerminalApps } =
      useTerminalApp();

    await detectTerminalApps();
    setTerminalApp("terminal");

    expect(localStorage.getItem(STORAGE_KEY)).toBe("terminal");
    expect(terminalApp.value).toBe("terminal");
    expect(terminalAppForLaunch.value).toBe("terminal");
  });

  it("探测失败时选项为空，UI 可据此隐藏设置项", async () => {
    mocks.invoke.mockRejectedValue(new Error("unsupported platform"));
    const { useTerminalApp } = await importModule();
    const { terminalApp, terminalAppOptions, detectTerminalApps } = useTerminalApp();

    await detectTerminalApps();
    expect(terminalAppOptions.value).toEqual([]);
    expect(terminalApp.value).toBeNull();
  });

  it("探测结果映射为带标签的选项", async () => {
    mocks.invoke.mockResolvedValue(["windows-terminal", "powershell", "cmd"]);
    const { useTerminalApp } = await importModule();
    const { terminalAppOptions, detectTerminalApps } = useTerminalApp();

    await detectTerminalApps();
    expect(terminalAppOptions.value).toEqual([
      { value: "windows-terminal", label: "Windows Terminal" },
      { value: "powershell", label: "PowerShell" },
      { value: "cmd", label: "CMD" },
    ]);
  });
});
