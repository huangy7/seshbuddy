import { describe, it, expect, vi, beforeEach } from "vitest";
import { useUsageStats, resetUsageStatsStore } from "./useUsageStats";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useUsageStats 单例状态与 SWR 缓存", () => {
  beforeEach(() => {
    resetUsageStatsStore();
    mockInvoke.mockReset();
  });

  it("初始状态应为空且未加载", () => {
    const { allRecords, recordsByCli, loading, loadError, lastFetchedAt } = useUsageStats();
    expect(allRecords.value).toEqual([]);
    expect(recordsByCli.value.size).toBe(0);
    expect(loading.value).toBe(false);
    expect(loadError.value).toBe("");
    expect(lastFetchedAt.value).toBeNull();
  });

  it("首次拉取数据成功并写入单例状态", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 100,
        project: "/projA",
      },
    ]);

    const { allRecords, loading, fetchUsage, lastFetchedAt } = useUsageStats();
    expect(loading.value).toBe(false);
    expect(allRecords.value).toHaveLength(0);

    const fetchPromise = fetchUsage(["claude"]);
    // 首次拉取时 loading 应该为 true
    expect(loading.value).toBe(true);

    await fetchPromise;
    expect(loading.value).toBe(false);
    expect(allRecords.value).toHaveLength(1);
    expect(allRecords.value[0].model).toBe("claude-3-5-sonnet");
    expect(lastFetchedAt.value).not.toBeNull();
  });

  it("已有时再次静默刷新使用 SWR 缓存数据，不置 loading 为 true", async () => {
    mockInvoke.mockResolvedValue([
      {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 100,
        project: "/projA",
      },
    ]);

    const store1 = useUsageStats();
    await store1.fetchUsage(["claude"]);

    const store2 = useUsageStats();
    expect(store2.allRecords.value).toHaveLength(1);
    expect(store2.loading.value).toBe(false);

    // 第二次拉取（静默 SWR 刷新）
    let loadingDuringFetch = false;
    mockInvoke.mockImplementationOnce(async () => {
      loadingDuringFetch = store2.loading.value;
      return [
        {
          date: "2026-08-29",
          model: "claude-3-5-sonnet",
          input_tokens: 200,
          output_tokens: 100,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          duration_ms: 120,
          project: "/projA",
        },
      ];
    });

    await store2.fetchUsage(["claude"]);
    expect(loadingDuringFetch).toBe(false);
    expect(store2.allRecords.value[0].date).toBe("2026-08-29");
  });

  it("force=true 强制刷新时应将 loading 置为 true", async () => {
    mockInvoke.mockResolvedValue([
      {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 100,
        project: "/projA",
      },
    ]);

    const store = useUsageStats();
    await store.fetchUsage(["claude"]);

    let loadingDuringFetch = false;
    mockInvoke.mockImplementationOnce(async () => {
      loadingDuringFetch = store.loading.value;
      return [];
    });

    await store.fetchUsage(["claude"], true);
    expect(loadingDuringFetch).toBe(true);
    expect(store.loading.value).toBe(false);
  });

  it("传入空 cliIds 时清空 recordsByCli 且不调用 invoke", async () => {
    mockInvoke.mockResolvedValue([
      {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 100,
        project: "/projA",
      },
    ]);

    const store = useUsageStats();
    await store.fetchUsage(["claude"]);
    expect(store.allRecords.value).toHaveLength(1);

    mockInvoke.mockReset();
    await store.fetchUsage([]);
    expect(store.allRecords.value).toHaveLength(0);
    expect(store.recordsByCli.value.size).toBe(0);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("当全部 CLI 请求失败时，设置 loadError 错误提示", async () => {
    mockInvoke.mockRejectedValue(new Error("Network Error"));

    const store = useUsageStats();
    await store.fetchUsage(["claude"]);

    expect(store.loadError.value).toBe("Failed to load usage data");
    expect(store.loading.value).toBe(false);
  });

  it("当部分 CLI 请求成功部分失败时，保留成功数据且不报错", async () => {
    mockInvoke.mockImplementation(async (_cmd: string, args: { cliId: string }) => {
      if (args.cliId === "claude") {
        return [
          {
            date: "2026-08-28",
            model: "claude-3-5-sonnet",
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            duration_ms: 100,
            project: "/projA",
          },
        ];
      }
      throw new Error("CLI not supported");
    });

    const store = useUsageStats();
    await store.fetchUsage(["claude", "codex"]);

    expect(store.allRecords.value).toHaveLength(1);
    expect(store.loadError.value).toBe("");
  });

  it("resetUsageStatsStore 应彻底重置所有单例状态", async () => {
    mockInvoke.mockResolvedValue([
      {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 100,
        project: "/projA",
      },
    ]);

    const store = useUsageStats();
    await store.fetchUsage(["claude"]);
    expect(store.allRecords.value).toHaveLength(1);
    expect(store.lastFetchedAt.value).not.toBeNull();

    resetUsageStatsStore();
    expect(store.allRecords.value).toHaveLength(0);
    expect(store.recordsByCli.value.size).toBe(0);
    expect(store.lastFetchedAt.value).toBeNull();
    expect(store.loading.value).toBe(false);
    expect(store.loadError.value).toBe("");
  });
});
