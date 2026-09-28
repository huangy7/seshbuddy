import { ref, shallowRef } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// 统计 watch 的注册与释放：本文件验证的是「watcher 生命周期」本身。
// 泄漏在行为上观察不到 —— 被保活的 watcher 已因代次校验失效、不会影响结果，
// 只是永远不释放，所以必须直接数 watcher。
const stopped = vi.fn();
const activeWatchers = new Set<number>();
let nextWatcherId = 0;

vi.mock("vue", async (importOriginal) => {
  const actual = await importOriginal<typeof import("vue")>();
  return {
    ...actual,
    watch: (...args: Parameters<typeof actual.watch>) => {
      const id = nextWatcherId++;
      activeWatchers.add(id);
      const stop = actual.watch(...args);
      return () => {
        if (activeWatchers.delete(id)) stopped(id);
        stop();
      };
    },
  };
});

const streamItems = shallowRef<string[]>([]);
const streamDone = ref<{ total: number } | null>(null);
const streamError = ref<Error | null>(null);
const streamLoading = ref(false);
const startMock = vi.fn();
const cancelMock = vi.fn();

vi.mock("./useStreamingLoad", () => ({
  useStreamingLoad: () => ({
    items: streamItems,
    done: streamDone,
    error: streamError,
    loading: streamLoading,
    start: startMock,
    cancel: cancelMock,
  }),
}));

const { useStreamingCollection } = await import("./useStreamingCollection");

describe("useStreamingCollection watcher lifecycle", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    stopped.mockClear();
    activeWatchers.clear();
    nextWatcherId = 0;
    streamItems.value = [];
    streamDone.value = null;
    streamError.value = null;
    streamLoading.value = false;
    startMock.mockReset();
    cancelMock.mockReset();
    startMock.mockImplementation(async () => {
      streamLoading.value = true;
      streamItems.value = [];
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("keeps watchers alive after the completion fallback so late results still land", async () => {
    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
      slowThresholdMs: 300,
    });

    const refreshPromise = collection.refresh({}, { atomicSwap: true });
    await Promise.resolve();

    // 越过 10 秒兜底：扫描仍在跑，watcher 必须保活
    await vi.advanceTimersByTimeAsync(11_000);
    await refreshPromise;
    expect(activeWatchers.size).toBeGreaterThan(0);
    expect(stopped).not.toHaveBeenCalled();

    // 迟到的结果仍要落地（这正是保活的目的）
    streamItems.value = ["fresh-1"];
    streamDone.value = { total: 1 };
    await Promise.resolve();
    expect(collection.items.value).toEqual(["fresh-1"]);

    // 流终结后 watcher 应当被释放
    expect(activeWatchers.size).toBe(0);
  });

  it("releases the kept-alive watchers on a later cancel", async () => {
    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
      slowThresholdMs: 300,
    });

    const refreshPromise = collection.refresh({}, { atomicSwap: true });
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(11_000);
    await refreshPromise;
    expect(activeWatchers.size).toBeGreaterThan(0);

    // 兜底超时后 watcher 仍活着，此时 cancel 必须能释放它们；
    // 若抢占句柄被 finally 提前清空，这批 watcher 将永远无人释放
    collection.cancel();

    expect(activeWatchers.size).toBe(0);
    expect(stopped).toHaveBeenCalled();
  });
});
