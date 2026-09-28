import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

const { useAutoIndex, AUTO_INDEX_INTERVAL_MAP } = await import("./useAutoIndex");

describe("useAutoIndex composable", () => {
  beforeEach(() => {
    storage.clear();
    mocks.invoke.mockReset();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("loads default interval as off when not configured", () => {
    const { autoIndexInterval } = useAutoIndex();
    expect(autoIndexInterval.value).toBe("off");
  });

  it("persists interval changes to localStorage", () => {
    const { autoIndexInterval, setAutoIndexInterval } = useAutoIndex();
    setAutoIndexInterval("30m");
    expect(autoIndexInterval.value).toBe("30m");
    expect(storage.get("seshbuddy-auto-index-interval")).toBe("30m");
  });

  it("triggers silent incremental sync periodically based on interval", async () => {
    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_search_index_status") {
        return { ready: false, requiresConfirmation: false, missingSessions: 1 };
      }
      if (cmd === "ensure_search_index_ready") return undefined;
      return undefined;
    });

    const { setAutoIndexInterval, startAutoIndexScheduler, stopAutoIndexScheduler } = useAutoIndex();
    setAutoIndexInterval("10m");
    const getVisibleCliIds = () => ["claude", "codex"];
    const isBusy = () => false;

    startAutoIndexScheduler(getVisibleCliIds, isBusy);

    // Fast-forward by 10 minutes (600,000 ms)
    await vi.advanceTimersByTimeAsync(AUTO_INDEX_INTERVAL_MAP["10m"] + 10);

    expect(mocks.invoke).toHaveBeenCalledWith("get_search_index_status", { cliId: "claude" });
    expect(mocks.invoke).toHaveBeenCalledWith("ensure_search_index_ready", { cliId: "claude" });
    expect(mocks.invoke).toHaveBeenCalledWith("get_search_index_status", { cliId: "codex" });
    expect(mocks.invoke).toHaveBeenCalledWith("ensure_search_index_ready", { cliId: "codex" });

    stopAutoIndexScheduler();
  });

  it("skips silent incremental sync when interval is off", async () => {
    const { setAutoIndexInterval, runSilentIncrementalSync } = useAutoIndex();
    setAutoIndexInterval("off");
    await runSilentIncrementalSync(["claude"]);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("skips silent incremental sync when missingSessions exceeds threshold", async () => {
    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_search_index_status") {
        return { ready: false, requiresConfirmation: false, missingSessions: 10 };
      }
      return undefined;
    });

    const { setAutoIndexInterval, runSilentIncrementalSync } = useAutoIndex();
    setAutoIndexInterval("10m");
    await runSilentIncrementalSync(["claude"]);

    expect(mocks.invoke).toHaveBeenCalledWith("get_search_index_status", { cliId: "claude" });
    expect(mocks.invoke).not.toHaveBeenCalledWith("ensure_search_index_ready", { cliId: "claude" });
  });

  it("skips silent incremental sync when status requires confirmation", async () => {
    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_search_index_status") {
        return { ready: false, requiresConfirmation: true, missingSessions: 2 };
      }
      return undefined;
    });

    const { setAutoIndexInterval, runSilentIncrementalSync } = useAutoIndex();
    setAutoIndexInterval("10m");
    await runSilentIncrementalSync(["claude"]);

    expect(mocks.invoke).toHaveBeenCalledWith("get_search_index_status", { cliId: "claude" });
    expect(mocks.invoke).not.toHaveBeenCalledWith("ensure_search_index_ready", { cliId: "claude" });
  });

  it("skips scheduled run when busy", async () => {
    const { startAutoIndexScheduler, stopAutoIndexScheduler } = useAutoIndex();
    const getVisibleCliIds = () => ["claude"];
    let busy = true;
    const isBusy = () => busy;

    startAutoIndexScheduler(getVisibleCliIds, isBusy);

    await vi.advanceTimersByTimeAsync(AUTO_INDEX_INTERVAL_MAP["10m"]);
    expect(mocks.invoke).not.toHaveBeenCalled();

    stopAutoIndexScheduler();
  });
});
