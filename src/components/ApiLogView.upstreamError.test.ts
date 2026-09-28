import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { setLocale } from "../i18n";
import type { SessionTrafficSummary, TrafficDetail, TrafficSummary } from "../composables/useProxy";

/**
 * 上游失败的正文提示**渲染**证据：proxy 只回事实 `error_kind`（今天只有 `"upstream"`），
 * 句子由组件自己的 computed 按当前语言渲染；`res_body` 是**第三方原文**，原样进 `{detail}` 槽位。
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
  status: 502,
  res_size: 1234,
  duration_ms: 30,
  input_tokens: null,
  output_tokens: null,
  cache_read_tokens: null,
  cache_creation_tokens: null,
  tool_names: [],
  has_skill_call: false,
};

/** 第三方原文：一个既不是 JSON 也不是 SSE 的普通错误串，原样展示才看得见差别 */
const THIRD_PARTY_TEXT = "connection refused";

const DETAIL: TrafficDetail = {
  id: RID,
  timestamp: "2026-09-25T00:00:00Z",
  method: "POST",
  path: "/v1/messages",
  req_headers: "{}",
  req_body: "{}",
  req_size: 10,
  status: 502,
  res_headers: "{}",
  // 上游失败：正文是第三方原文本身，事实在 error_kind 里
  res_body: THIRD_PARTY_TEXT,
  compression: null,
  error_kind: "upstream",
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

async function mountWithDetail() {
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

function bodyText(wrapper: Awaited<ReturnType<typeof mountWithDetail>>): string {
  return (wrapper.vm as unknown as { detailBodyText: string }).detailBodyText;
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation((command: string) => Promise.resolve(respondTo(command)));
});

afterEach(() => setLocale("en"));

describe("上游失败的正文提示按 error_kind 由前端渲染", () => {
  it("阳性对照：error_kind 为 upstream 时四种语言各渲染一句，第三方原文原样保留", async () => {
    const wrapper = await mountWithDetail();

    const expected: Record<string, string> = {
      en: "[Upstream request failed: connection refused]",
      zh: "[上游请求失败: connection refused]",
      ja: "[上流リクエストに失敗しました: connection refused]",
      de: "[Upstream-Anfrage fehlgeschlagen: connection refused]",
    };

    for (const locale of ["en", "zh", "ja", "de"] as const) {
      setLocale(locale);
      await wrapper.vm.$nextTick();
      const text = bodyText(wrapper);
      // 句子随当前语言变
      expect(text, locale).toBe(expected[locale]);
      // 第三方原文逐字保留（R3：第三方文本永不翻译）
      expect(text, locale).toContain(THIRD_PARTY_TEXT);
    }
  });

  it("阴性对照：error_kind 为 null 时不出现提示句，正文照常渲染", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "proxy_get_detail" ? { ...DETAIL, error_kind: null } : respondTo(command),
      ),
    );
    const wrapper = await mountWithDetail();

    // 同一个 res_body 在阳性对照里被包进提示句；这里必须原样呈现——
    // 「恒显示提示句」与「只在 upstream 时显示」由此可区分。
    expect(bodyText(wrapper)).toBe(THIRD_PARTY_TEXT);
    expect(bodyText(wrapper)).not.toContain("Upstream request failed");
  });

  it("阴性对照：error_kind 为未知值时同样不渲染提示句（兜底是原样展示正文）", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "proxy_get_detail" ? { ...DETAIL, error_kind: "timeout" } : respondTo(command),
      ),
    );
    const wrapper = await mountWithDetail();

    expect(bodyText(wrapper)).toBe(THIRD_PARTY_TEXT);
  });
});
