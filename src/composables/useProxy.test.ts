import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { useProxy, type SessionTrafficSummary } from "./useProxy";

const invokeMock = vi.fn();
const listenMock = vi.fn(() => Promise.resolve(() => {}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (ev: unknown) => void) => listenMock(event, handler),
}));

// src-tauri/src/proxy.rs 的 open_traffic_db() 在 traffic.db 尚未创建时发出的错误码。
// 它是跨进程协议状态（「代理还没跑过」），不是界面文案：早先这里按后端错误原文做子串匹配，
// 语言包里为此存了一份四语刻意不译的副本（`common.error.backendMarker.trafficDbMissing`）——
// 译文一变（或换门语言）分支就静默失效，用户会看到本该被抑制的「错误」。
const BACKEND_TRAFFIC_DB_MISSING = "proxy.traffic_db_missing";
// 不写成 `new URL("../../…", import.meta.url)`：那个字面形状是 Vite 的资源引用语法，
// 会被改写成非 file: 的模块 URL，fileURLToPath 直接抛「The URL must be of scheme file」。
// 经一个变量转手可避开该改写，实测取到的是本文件真实路径。
const TEST_FILE_URL = import.meta.url;
const BACKEND_PROXY_RS = join(
  fileURLToPath(new URL("../../", TEST_FILE_URL)),
  "src-tauri/src/proxy.rs",
);

/**
 * 走一遍 loadSessions 的错误分支，返回它把什么显示了出来（空串 = 这条错误被抑制了）。
 *
 * 拒绝从 `listen` 的注册抛出：那是 `loadSessions` 的 catch 唯一能收到的输入
 * （流内部的 invoke 拒绝会被 `useStreamingLoad` 收进 `stream.error`，不经这里）。
 */
async function errorShownFor(rejection: unknown): Promise<string> {
  listenMock.mockRejectedValueOnce(rejection);
  const proxy = useProxy("claude");
  await proxy.loadSessions();
  return proxy.error.value;
}

/** 带 code 的拒绝：形状与 `invokeApp` 包装后抛出的 `Error` 一致（message 是渲染好的文案）。 */
function codedRejection(code: string, message: string): Error {
  return Object.assign(new Error(message), { code });
}

describe("trafficDbMissing 协议标记", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(() => Promise.resolve(() => {}));
  });

  it("后端仍在发出这个 code（改后端也会红）", () => {
    const rust = readFileSync(BACKEND_PROXY_RS, "utf8");
    expect(
      rust,
      `${BACKEND_PROXY_RS} 里已找不到 coded("${BACKEND_TRAFFIC_DB_MISSING}")`,
    ).toContain(`coded("${BACKEND_TRAFFIC_DB_MISSING}")`);
  });

  // 前一条只证明「后端仍发这个 code」：把 loadSessions 里那个 if 分支删掉，它照样全绿。
  // 故这一条真跑一遍 loadSessions——loadSessions 的 catch 是这条标记唯一的用武之地。
  // message 刻意用渲染好的英文文案（不等于 code）：判据若退回「从 message 里找子串」，
  // 第一条就会红。
  it("loadSessions 仍按这个 code 抑制（删掉那个 if 分支就会红）", async () => {
    // 命中 code：属于「代理还没跑过」的正常状态，不设 error。
    expect(
      await errorShownFor(codedRejection(BACKEND_TRAFFIC_DB_MISSING, "Traffic database does not exist")),
    ).toBe("");
    // 反向一：别的 code 必须原样透传，否则这条分支会吞掉所有别的错误。
    expect(await errorShownFor(codedRejection("nope.missing", "查询失败: no such table: traffic"))).toBe(
      "查询失败: no such table: traffic",
    );
    // 反向二：只有文案没有 code 不再命中——判据已从「文案子串」改成「code」，
    // 这条断言就是耦合被拆掉的证据（旧实现下它会红）。
    expect(await errorShownFor(new Error("流量数据库不存在"))).toBe("流量数据库不存在");
  });
});

/**
 * WS 完成事件的合并：`updateSessionOnComplete` 展开 `...old` 复用服务端给的**标签对象**。
 *
 * 标签改成结构化之后这里必须仍然合得上——标签是 `session_id` 的纯函数，
 * 而合并按 `session_id` 找行，所以复用旧值不会串行。本用例把这条推理钉成断言。
 */
class FakeWebSocket {
  static last: FakeWebSocket | null = null;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {
    FakeWebSocket.last = this;
  }
  close() {}
}

const SID = "3f2a1b9c-4d5e-6f70-8192-a3b4c5d6e7f8";

function summary(overrides: Partial<SessionTrafficSummary> = {}): SessionTrafficSummary {
  return {
    session_id: SID,
    display_name: { kind: "session", session_id: SID },
    project_path: "/tmp/project",
    request_count: 1,
    total_req_size: 10,
    total_res_size: 20,
    total_duration_ms: 30,
    first_timestamp: "2026-09-25T00:00:00Z",
    last_timestamp: "2026-09-25T00:00:00Z",
    ok_count: 1,
    error_count: 0,
    ...overrides,
  };
}

function completeEvent(sessionId: string | null) {
  return {
    data: JSON.stringify({
      type: "complete",
      id: "req-1",
      status: 200,
      duration_ms: 5,
      req_size: 1,
      res_size: 2,
      session_id: sessionId,
    }),
  };
}

function connectedProxy() {
  vi.stubGlobal("WebSocket", FakeWebSocket);
  const proxy = useProxy("claude");
  proxy.connectWs(1234);
  return proxy;
}

describe("WS 完成事件合并时保留结构化标签", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(() => Promise.resolve(() => {}));
    FakeWebSocket.last = null;
  });

  it("命中已有会话：计数累加，标签原样复用（`...old`）", () => {
    const proxy = connectedProxy();
    proxy.sessions.value = [summary()];

    FakeWebSocket.last!.onmessage!(completeEvent(SID));

    expect(proxy.sessions.value).toHaveLength(1);
    expect(proxy.sessions.value[0].request_count).toBe(2);
    expect(proxy.sessions.value[0].display_name).toEqual({ kind: "session", session_id: SID });
  });

  it("未关联桶：按 session_id === null 找到同一行，标签仍是 Ungrouped", () => {
    const proxy = connectedProxy();
    proxy.sessions.value = [
      summary(),
      summary({ session_id: null, display_name: { kind: "ungrouped" }, project_path: "" }),
    ];

    FakeWebSocket.last!.onmessage!(completeEvent(null));

    expect(proxy.sessions.value).toHaveLength(2);
    const bucket = proxy.sessions.value.find((s) => s.session_id === null)!;
    expect(bucket.request_count).toBe(2);
    expect(bucket.display_name).toEqual({ kind: "ungrouped" });
    // 未关联桶保持在末尾，真实会话仍在首位
    expect(proxy.sessions.value[1].session_id).toBeNull();
  });

  it("用户数据的标题也一并复用（合并不会把它换成标签）", () => {
    const proxy = connectedProxy();
    proxy.sessions.value = [
      summary({ display_name: { kind: "named", title: "优化会话加载性能" } }),
    ];

    FakeWebSocket.last!.onmessage!(completeEvent(SID));

    expect(proxy.sessions.value[0].display_name).toEqual({
      kind: "named",
      title: "优化会话加载性能",
    });
  });
});
