import { describe, it, expect, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("./useSessions", async () => {
  const { ref } = await import("vue");
  const searchIndexProgress = ref<any>(null);
  return {
    useSessions: () => ({ searchIndexProgress }),
    __searchIndexProgress: searchIndexProgress,
  };
});

const { useIndexRebuildQueue } = await import("./useIndexRebuildQueue");
const sessionsMock = (await import("./useSessions")) as any;

function setProgress(payload: any | null) {
  sessionsMock.__searchIndexProgress.value = payload;
}

describe("useIndexRebuildQueue", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
    setProgress(null);
  });

  it("依次对每个 CLI 执行 clear → refresh → ensure", async () => {
    const { startRebuildAll } = useIndexRebuildQueue();
    const calls: string[] = [];
    mocks.invoke.mockImplementation(async (cmd: string, args: any) => {
      calls.push(`${cmd}:${args.cliId}`);
    });

    await startRebuildAll([
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
    ]);

    expect(calls).toEqual([
      "clear_session_index:claude",
      "refresh_session_list_index:claude",
      "ensure_search_index_ready:claude",
      "clear_session_index:codex",
      "refresh_session_list_index:codex",
      "ensure_search_index_ready:codex",
    ]);
  });

  it("运行期间暴露当前 CLI 与进度计数，结束后复位", async () => {
    const { startRebuildAll, running, currentCliId, currentIndex, total } =
      useIndexRebuildQueue();
    const snapshots: Array<[string | undefined, number, number]> = [];
    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "ensure_search_index_ready") {
        snapshots.push([currentCliId.value, currentIndex.value, total.value]);
      }
    });

    expect(running.value).toBe(false);

    const done = startRebuildAll([
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
      { id: "gemini", name: "Gemini" },
    ]);
    expect(running.value).toBe(true);
    await done;

    expect(snapshots).toEqual([
      ["claude", 0, 3],
      ["codex", 1, 3],
      ["gemini", 2, 3],
    ]);
    expect(running.value).toBe(false);
    expect(currentCliId.value).toBeUndefined();
  });

  it("isQueued 仅对排在当前之后的 CLI 返回 true", async () => {
    const { startRebuildAll, isQueued } = useIndexRebuildQueue();
    expect(isQueued("codex")).toBe(false);

    let releaseFirst: () => void;
    const gate = new Promise<void>((resolve) => (releaseFirst = resolve));
    mocks.invoke.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === "ensure_search_index_ready" && args.cliId === "claude") {
        await gate;
      }
    });

    const done = startRebuildAll([
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
      { id: "gemini", name: "Gemini" },
    ]);
    // 让循环推进到 claude 的 ensure 阶段
    await Promise.resolve();
    await Promise.resolve();

    expect(isQueued("claude")).toBe(false); // 当前项
    expect(isQueued("codex")).toBe(true);
    expect(isQueued("gemini")).toBe(true);
    expect(isQueued("dsh")).toBe(false); // 不在队列

    releaseFirst!();
    await done;
    expect(isQueued("codex")).toBe(false); // 结束后无队列
  });

  it("overallPercent 按 (已完成数 + 当前CLI进度) / 总数 加权", async () => {
    const { startRebuildAll, overallPercent } = useIndexRebuildQueue();
    let releaseFirst: () => void;
    const gate = new Promise<void>((resolve) => (releaseFirst = resolve));
    mocks.invoke.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === "ensure_search_index_ready" && args.cliId === "claude") {
        await gate;
      }
    });

    const done = startRebuildAll([
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
    ]);
    await Promise.resolve();
    await Promise.resolve();

    // 当前 CLI (claude) 扫描到 50%（scanning 相位映射 0-90 → 45）
    setProgress({ cliId: "claude", phase: "scanning", current: 1, total: 2, processedBytes: 50, totalBytes: 100 });
    // (0 + 45/100) / 2 = 22.5 → 23
    expect(overallPercent.value).toBe(23);

    // 其他 CLI 的进度事件不计入
    setProgress({ cliId: "codex", phase: "scanning", current: 1, total: 1, processedBytes: 100, totalBytes: 100 });
    expect(overallPercent.value).toBe(23);

    releaseFirst!();
    await done;
  });

  it("overallPercent 单调不回落（done 帧清空进度时保持高水位）", async () => {
    const { startRebuildAll, overallPercent } = useIndexRebuildQueue();
    let releaseFirst: () => void;
    const gate = new Promise<void>((resolve) => (releaseFirst = resolve));
    mocks.invoke.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === "ensure_search_index_ready" && args.cliId === "claude") {
        await gate;
      }
    });

    const done = startRebuildAll([
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
    ]);
    await Promise.resolve();
    await Promise.resolve();

    // claude 提交完成 → 100%，加权 (0 + 1) / 2 = 50
    setProgress({ cliId: "claude", phase: "committed", current: 10, total: 10 });
    expect(overallPercent.value).toBe(50);

    // done 帧：useSessions 会把进度置 null，高水位保持 50 不归零
    setProgress(null);
    expect(overallPercent.value).toBe(50);

    releaseFirst!();
    await done;
    expect(overallPercent.value).toBe(0); // 结束后复位
  });

  it("任一 CLI 失败即中断并向调用方抛错，状态复位", async () => {
    const { startRebuildAll, running, currentCliId } = useIndexRebuildQueue();
    mocks.invoke.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === "ensure_search_index_ready" && args.cliId === "claude") {
        throw new Error("disk full");
      }
    });

    await expect(
      startRebuildAll([
        { id: "claude", name: "Claude Code" },
        { id: "codex", name: "Codex" },
      ]),
    ).rejects.toThrow("disk full");

    // codex 未被执行
    expect(
      mocks.invoke.mock.calls.some(([cmd, args]: any[]) => args?.cliId === "codex"),
    ).toBe(false);
    expect(running.value).toBe(false);
    expect(currentCliId.value).toBeUndefined();
  });

  it("运行期间重复调用 startRebuildAll 直接忽略", async () => {
    const { startRebuildAll } = useIndexRebuildQueue();
    let release: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    mocks.invoke.mockImplementation(async () => gate);

    const first = startRebuildAll([{ id: "claude", name: "Claude Code" }]);
    await Promise.resolve();
    const second = startRebuildAll([{ id: "codex", name: "Codex" }]);
    await second;

    expect(
      mocks.invoke.mock.calls.some(([cmd, args]: any[]) => args?.cliId === "codex"),
    ).toBe(false);

    release!();
    await first;
  });
});
