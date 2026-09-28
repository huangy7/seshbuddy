import { nextTick, ref, shallowRef } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStreamingCollection } from "./useStreamingCollection";

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

describe("useStreamingCollection SWR preservation", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    streamItems.value = [];
    streamDone.value = null;
    streamError.value = null;
    streamLoading.value = false;
    startMock.mockReset();
    cancelMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not wipe out existing items when refresh is slow (> slowThresholdMs)", async () => {
    startMock.mockImplementation(async () => {
      streamLoading.value = true;
      streamItems.value = [];
    });

    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
      slowThresholdMs: 300,
    });

    // Populate initial items from a previous successful load
    collection.items.value = ["session-1", "session-2", "session-3"];
    collection.totalItems.value = 3;

    // Start a new refresh
    const refreshPromise = collection.refresh();
    await nextTick();

    // Advance timers past slowThresholdMs (e.g. 500ms on a slow machine)
    await vi.advanceTimersByTimeAsync(500);

    // CRITICAL ASSERTION: Existing items MUST be preserved (SWR), NOT wiped to []!
    expect(collection.items.value).toEqual(["session-1", "session-2", "session-3"]);
    expect(collection.totalItems.value).toBe(3);
    // Loading indicator is NOT shown because we already have items to display
    expect(collection.showLoadingIndicator.value).toBe(false);

    // Now simulated new chunk arrives
    streamItems.value = ["new-session-1"];
    await nextTick();

    expect(collection.items.value).toEqual(["new-session-1"]);

    // Done arrives
    streamDone.value = { total: 1 };
    await refreshPromise;

    expect(collection.items.value).toEqual(["new-session-1"]);
    expect(collection.isRefreshing.value).toBe(false);
  });

  it("keeps the directory stable until an atomic refresh has its complete result", async () => {
    startMock.mockImplementation(async (...args: any[]) => {
      streamLoading.value = true;
      // useStreamingLoad's atomic mode keeps the old stream items untouched
      // until its done callback commits the staged result.
      expect(args[3]).toEqual({ atomicSwap: true });
    });

    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
    });
    collection.items.value = ["old-1", "old-2"];
    collection.totalItems.value = 2;

    const refreshPromise = collection.refresh({}, { silent: true });
    await nextTick();
    await vi.advanceTimersByTimeAsync(500);

    // No partial chunk has been committed, so the visible tree is unchanged.
    expect(collection.items.value).toEqual(["old-1", "old-2"]);

    // This assignment represents useStreamingLoad's single done-time commit.
    streamItems.value = ["new-1", "new-2", "new-3"];
    await nextTick();
    expect(collection.items.value).toEqual(["new-1", "new-2", "new-3"]);

    streamDone.value = { total: 3 };
    await refreshPromise;
    expect(collection.isRefreshing.value).toBe(false);
  });

  it("keeps the latest refresh busy when an older refresh is superseded", async () => {
    startMock.mockResolvedValue(undefined);

    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
    });

    const firstRefresh = collection.refresh();
    await nextTick();
    const secondRefresh = collection.refresh();
    await nextTick();
    await Promise.resolve();

    expect(collection.isRefreshing.value).toBe(true);

    streamDone.value = { total: 0 };
    await Promise.all([firstRefresh, secondRefresh]);
    expect(collection.isRefreshing.value).toBe(false);
  });

  it("clears items when scan finishes (done) with 0 items", async () => {
    startMock.mockImplementation(async () => {
      streamLoading.value = true;
      streamItems.value = [];
    });

    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
      slowThresholdMs: 300,
    });

    collection.items.value = ["old-session"];
    collection.totalItems.value = 1;

    const refreshPromise = collection.refresh();
    await nextTick();

    // Done arrives with NO chunks received at all (directory was emptied)
    streamDone.value = { total: 0 };
    await refreshPromise;

    expect(collection.items.value).toEqual([]);
    expect(collection.totalItems.value).toBe(0);
  });

  it("preserves items when an error occurs during refresh", async () => {
    startMock.mockImplementation(async () => {
      streamLoading.value = true;
      streamItems.value = [];
    });

    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
      slowThresholdMs: 300,
    });

    collection.items.value = ["keep-me"];
    collection.totalItems.value = 1;

    const refreshPromise = collection.refresh();
    await nextTick();

    streamError.value = new Error("Network timeout");
    await refreshPromise;

    expect(collection.items.value).toEqual(["keep-me"]);
    expect(collection.error.value?.message).toBe("Network timeout");
  });

  // 兜底计时器（10s）只是解除 await 挂起，不代表扫描结束：扫描慢于兜底阈值时，
  // 迟到的结果仍必须落地。回归场景是换代后的首轮全量重解析 —— 事件要等整个 CLI 的
  // 列表构建完才发出，超过 10s 很常见，此刻若已拆掉 watcher，atomicSwap 模式下
  // items 永不提交，用户看到的是永远停在旧快照的侧边栏。
  it("commits a scan result that arrives after the completion fallback fires", async () => {
    startMock.mockImplementation(async () => {
      streamLoading.value = true;
      streamItems.value = [];
    });

    const collection = useStreamingCollection<string, { total: number }>({
      command: "test_cmd",
      topic: "test_topic",
      slowThresholdMs: 300,
    });

    collection.items.value = ["stale-1"];
    collection.totalItems.value = 1;

    const refreshPromise = collection.refresh({}, { atomicSwap: true });
    await nextTick();

    // 越过 10 秒兜底：refresh 应当已返回，但结果还没到
    await vi.advanceTimersByTimeAsync(11_000);
    await refreshPromise;

    // 扫描此刻才真正结束
    streamItems.value = ["fresh-1", "fresh-2"];
    streamDone.value = { total: 2 };
    await nextTick();

    expect(collection.items.value).toEqual(["fresh-1", "fresh-2"]);
    expect(collection.totalItems.value).toBe(2);
    expect(collection.done.value).toEqual({ total: 2 });
  });
});
