import { describe, expect, it, vi, beforeEach } from "vitest";
import { useStreamingLoad } from "./useStreamingLoad";

// Mock Tauri invoke and listen
const invokeMock = vi.fn();
const listeners = new Map<string, (ev: any) => void>();
const listenMock = vi.fn((event: string, handler: (ev: any) => void) => {
  listeners.set(event, handler);
  return Promise.resolve(() => {
    listeners.delete(event);
  });
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: any[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: any) => listenMock(event, handler),
}));

describe("useStreamingLoad", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockClear();
    listeners.clear();
  });

  it("settles start when done arrives before invoke resolves", async () => {
    vi.useFakeTimers();
    let resolveInvoke!: () => void;
    invokeMock.mockImplementationOnce(
      () => new Promise<void>((resolve) => { resolveInvoke = resolve; }),
    );

    const stream = useStreamingLoad<string, { total: number }>();
    const startPromise = stream.start("load_data", "test-topic", {});
    await Promise.resolve();
    await Promise.resolve();

    emitEvent("test-topic", "done", { total: 0 });

    const settledBeforeInvokeReturns = Promise.race([
      startPromise.then(() => true),
      new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 10)),
    ]);
    await vi.advanceTimersByTimeAsync(10);
    resolveInvoke();

    expect(await settledBeforeInvokeReturns).toBe(true);
    vi.useRealTimers();
  });

  function emitEvent(topic: string, type: "chunk" | "done" | "error", payload: any) {
    for (const [evt, handler] of listeners.entries()) {
      if (evt.startsWith(`${topic}:`) && evt.endsWith(`:${type}`)) {
        handler({ payload });
        return;
      }
    }
    throw new Error(`Listener for ${topic} ${type} not found among: ${Array.from(listeners.keys()).join(", ")}`);
  }

  it("loads stream in standard mode: resets items, appends chunks, completes on done", async () => {
    const stream = useStreamingLoad<string, { total: number }>();
    stream.items.value = ["old-item"];

    invokeMock.mockResolvedValueOnce(undefined);

    const startPromise = stream.start("load_data", "test-topic", {});
    await startPromise;

    // Standard mode resets items immediately
    expect(stream.items.value).toEqual([]);
    expect(stream.loading.value).toBe(true);

    // Emit chunk 1
    emitEvent("test-topic", "chunk", ["a", "b"]);
    expect(stream.items.value).toEqual(["a", "b"]);

    // Emit chunk 2
    emitEvent("test-topic", "chunk", ["c"]);
    // Emit done
    emitEvent("test-topic", "done", { total: 3 });

    const result = await stream.waitForCompletion();
    expect(result).toEqual({ total: 3 });
    expect(stream.items.value).toEqual(["a", "b", "c"]);
    expect(stream.loading.value).toBe(false);
    expect(stream.done.value).toEqual({ total: 3 });
  });

  it("loads stream in atomicSwap (SWR) mode: preserves old items until done", async () => {
    const stream = useStreamingLoad<string, { total: number }>();
    stream.items.value = ["message-1", "message-2"];

    invokeMock.mockResolvedValueOnce(undefined);

    await stream.start("load_data", "test-topic", {}, { atomicSwap: true });

    // SWR mode preserves old items on start!
    expect(stream.items.value).toEqual(["message-1", "message-2"]);
    expect(stream.loading.value).toBe(true);

    // Chunk arrives: items still show old items to prevent UI flash
    emitEvent("test-topic", "chunk", ["message-1", "message-2", "message-3"]);
    expect(stream.items.value).toEqual(["message-1", "message-2"]);

    // Another chunk arrives
    emitEvent("test-topic", "chunk", ["message-4"]);
    expect(stream.items.value).toEqual(["message-1", "message-2"]);

    // Done arrives: atomic replacement happens!
    emitEvent("test-topic", "done", { total: 4 });

    const doneResult = await stream.waitForCompletion();
    expect(doneResult).toEqual({ total: 4 });
    expect(stream.items.value).toEqual(["message-1", "message-2", "message-3", "message-4"]);
    expect(stream.loading.value).toBe(false);
  });

  it("preserves old items when error occurs in atomicSwap mode", async () => {
    const stream = useStreamingLoad<string, { total: number }>();
    stream.items.value = ["existing-turn"];

    invokeMock.mockResolvedValueOnce(undefined);

    await stream.start("load_data", "test-topic", {}, { atomicSwap: true });
    expect(stream.items.value).toEqual(["existing-turn"]);

    // Error arrives
    emitEvent("test-topic", "error", "Disk read timeout");

    const doneResult = await stream.waitForCompletion();
    expect(doneResult).toBeNull();
    // Old view is preserved!
    expect(stream.items.value).toEqual(["existing-turn"]);
    expect(stream.error.value?.message).toBe("Disk read timeout");
    expect(stream.loading.value).toBe(false);
  });

  // 通道 2 的载荷只有两种形状：`{ code, params }`（后端已改造）与裸字符串（R1 迁移期）。
  // 两条都要有控制，否则「只认一种」会在另一种上退化成 `[object Object]` 或空文案。
  it("error 事件载荷是 { code, params } 时渲染当前语言的语言包文案", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const stream = useStreamingLoad<string, { total: number }>();
    await stream.start("load_data", "test-topic", {});

    emitEvent("test-topic", "error", {
      code: "session.file_missing",
      params: { path: "/tmp/gone.jsonl" },
    });

    expect(stream.error.value?.message).toBe("Session file not found: /tmp/gone.jsonl");
    expect(stream.loading.value).toBe(false);
  });

  it("error 事件载荷是字符串时原样上屏（R1 迁移期形状）", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const stream = useStreamingLoad<string, { total: number }>();
    await stream.start("load_data", "test-topic", {});

    emitEvent("test-topic", "error", "会话文件不存在: /tmp/gone.jsonl");

    expect(stream.error.value?.message).toBe("会话文件不存在: /tmp/gone.jsonl");
    expect(stream.loading.value).toBe(false);
  });

  it("waitForCompletion resolves null on timeout rather than hanging indefinitely", async () => {
    vi.useFakeTimers();
    const stream = useStreamingLoad<string, { total: number }>();
    invokeMock.mockResolvedValueOnce(undefined);

    await stream.start("load_data", "test-topic", {});
    expect(stream.loading.value).toBe(true);

    const completionPromise = stream.waitForCompletion(500);
    await vi.advanceTimersByTimeAsync(600);

    const result = await completionPromise;
    expect(result).toBeNull();
    vi.useRealTimers();
  });

  it("resolves completion when stream is cancelled", async () => {
    const stream = useStreamingLoad<string, { total: number }>();
    invokeMock.mockResolvedValueOnce(undefined);

    await stream.start("load_data", "test-topic", {});
    const completionPromise = stream.waitForCompletion();

    stream.cancel();
    expect(await completionPromise).toBeNull();
    expect(stream.loading.value).toBe(false);
  });
});
