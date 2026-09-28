import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { setLocale } from "../i18n";
import { isSseBody } from "../utils/sse";
import { compressionNotice } from "../composables/useProxy";
import type { SessionTrafficSummary, TrafficDetail, TrafficSummary } from "../composables/useProxy";

/**
 * 压缩响应的正文提示**渲染**证据：后端不再往 `res_body` 里写中文标记，
 * 只回 `compression`（编码名）与 `res_size`；句子由组件自己的 computed 渲染。
 * 断言的是 `detailBodyText`——它正是传给 Monaco 查看器的那个字符串。
 */

const SID = "3f2a1b9c-4d5e-6f70-8192-a3b4c5d6e7f8";
const RID = "req-1";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("../monaco-workers", () => ({ monaco: { editor: {} } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ message: vi.fn() }));
vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({
    cliOptions: ref([{ id: "claude", name: "Claude", supportsApiLogs: true }]),
  }),
}));

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});
vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
  cb(0);
  return 0;
});
vi.stubGlobal("IntersectionObserver", class {
  observe() {}
  disconnect() {}
});

const ApiLogView = (await import("./ApiLogView.vue")).default;

const SESSION: SessionTrafficSummary = {
  session_id: SID,
  display_name: { kind: "session", session_id: SID },
  project_path: "/tmp/project",
  request_count: 1,
  total_req_size: 10,
  total_res_size: 20,
  total_duration_ms: 30,
  first_timestamp: "2026-09-25T00:00:00Z",
  last_timestamp: "2026-09-25T00:00:01Z",
  ok_count: 1,
  error_count: 0,
};

const TRAFFIC: TrafficSummary = {
  id: RID,
  timestamp: "2026-09-25T00:00:00Z",
  method: "POST",
  path: "/v1/messages",
  req_size: 10,
  status: 200,
  res_size: 1234,
  duration_ms: 30,
  input_tokens: null,
  output_tokens: null,
  cache_read_tokens: null,
  cache_creation_tokens: null,
  tool_names: [],
  has_skill_call: false,
};

const DETAIL: TrafficDetail = {
  id: RID,
  timestamp: "2026-09-25T00:00:00Z",
  method: "POST",
  path: "/v1/messages",
  req_headers: "{}",
  req_body: "{}",
  req_size: 10,
  status: 200,
  res_headers: "{}",
  // 压缩响应：正文未落库，只有 compression 与 res_size
  res_body: null,
  compression: "gzip",
  error_kind: null,
  res_size: 1234,
  duration_ms: 30,
};

function respondTo(command: string): unknown {
  switch (command) {
    case "proxy_get_traffic_sessions":
      return { items: [SESSION], total: 1 };
    case "proxy_get_session_traffic":
      return { items: [TRAFFIC], total: 1 };
    case "proxy_get_detail":
      return DETAIL;
    case "proxy_get_db_size":
      return 0;
    default:
      return null;
  }
}

async function mountWithCompressedDetail() {
  const wrapper = mount(ApiLogView, { props: { active: true, initialCliId: "claude" } });
  await flushPromises();
  await (wrapper.vm as unknown as { toggleSession: (s: SessionTrafficSummary) => Promise<void> })
    .toggleSession(SESSION);
  await flushPromises();
  // 切到响应体页签：提示只在响应侧渲染（请求侧仍是请求正文）
  (wrapper.vm as unknown as { detailTab: "request" | "response" }).detailTab = "response";
  await wrapper.vm.$nextTick();
  return wrapper;
}

function bodyText(wrapper: Awaited<ReturnType<typeof mountWithCompressedDetail>>): string {
  return (wrapper.vm as unknown as { detailBodyText: string }).detailBodyText;
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation((command: string) => Promise.resolve(respondTo(command)));
});

afterEach(() => setLocale("en"));

describe("压缩响应的正文提示由前端渲染", () => {
  it("四种语言各渲染一句，编码与字节数来自结构化字段", async () => {
    const wrapper = await mountWithCompressedDetail();

    expect(bodyText(wrapper)).toBe(
      "[Response body was compressed with gzip and stored without decompression; original size 1234 bytes]",
    );

    setLocale("zh");
    await wrapper.vm.$nextTick();
    expect(bodyText(wrapper)).toBe("[响应体经 gzip 压缩，未解压存储；原始大小 1234 字节]");

    setLocale("ja");
    await wrapper.vm.$nextTick();
    expect(bodyText(wrapper)).toBe(
      "[レスポンス本文は gzip で圧縮されており、非解凍のまま保存されています。元のサイズ 1234 バイト]",
    );

    setLocale("de");
    await wrapper.vm.$nextTick();
    expect(bodyText(wrapper)).toBe(
      "[Antworttext wurde mit gzip komprimiert und ohne Dekomprimierung gespeichert; ursprüngliche Größe 1234 Byte]",
    );
  });

  it("未压缩的响应照常走正文渲染（提示只在 compression 有值时出现）", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "proxy_get_detail"
          ? { ...DETAIL, res_body: '{"a":1}', compression: null }
          : respondTo(command),
      ),
    );
    const wrapper = await mountWithCompressedDetail();
    expect(bodyText(wrapper)).toBe('{\n  "a": 1\n}');
  });

  /**
   * 提示句不进 SSE 分支：`isSseBody` 认的是正文**形态**（`event:` / `data:` 开头），
   * 不是提示文本，所以无论提示句在哪种语言下长什么样都不该被判成 SSE 流。
   * 阳性对照在最后一条——证明这个判据不是「对任何输入都返回 false」。
   */
  it("提示句不被 isSseBody 判成 SSE（四语实测）", () => {
    for (const locale of ["en", "zh", "ja", "de"] as const) {
      setLocale(locale);
      const notice = compressionNotice(DETAIL);
      expect(notice, locale).not.toBe("");
      expect(isSseBody(notice), locale).toBe(false);
    }
    expect(isSseBody("data: {\"a\":1}")).toBe(true);
  });
});
