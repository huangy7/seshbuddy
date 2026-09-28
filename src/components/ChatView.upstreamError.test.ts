import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { setLocale } from "../i18n";

/**
 * 聊天时间线里 ⚡ 面板的**第三个渲染面**：`ChatView` 的 `apiBodyHtml()`。
 *
 * 它与 `ApiLogView` 读的是同一份流量记录（`proxy_find_by_timestamp` → 应用侧
 * `find_traffic_by_timestamp`，同一个列清单、同样带 `error_kind`），所以同一次上游失败
 * 必须在两处渲染出同一句话。断言的是 `apiBodyHtml()`——它正是传给
 * `ChatApiDetailPanel` 的那个 HTML。
 */

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(event, handler);
    return () => mocks.listeners.delete(event);
  }),
}));

vi.mock("../composables/useSessions", async () => {
  const { ref: vueRef } = await import("vue");
  return {
    useSessions: () => ({
      exportSession: vi.fn(),
      projects: vueRef([]),
    }),
  };
});

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
  callback(0);
  return 0;
});

vi.stubGlobal("IntersectionObserver", class {
  observe() {}
  disconnect() {}
});

const ChatView = (await import("./ChatView.vue")).default;

const TIMESTAMP = "2026-08-27T08:00:00Z";

/** 第三方原文：既不是 JSON 也不是 SSE 的普通错误串，原样展示才看得见差别 */
const THIRD_PARTY_TEXT = "connection refused";

const DETAIL = {
  id: "req-1",
  timestamp: TIMESTAMP,
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
  error_kind: "upstream" as string | null,
  res_size: 1234,
  duration_ms: 30,
};

function mountChatView() {
  return mount(ChatView, {
    props: {
      sessionPath: "/sessions/first.jsonl",
      encodedDir: "project",
      active: true,
      showSearch: false,
      bookmarks: [],
      sessionId: "first",
      projectRoot: "/workspace/project",
      autoFollow: false,
      filterUser: true,
      filterAssistant: true,
      filterTool: true,
      filterThinking: true,
      cliId: "claude",
    },
    global: {
      stubs: {
        LoadingSkeleton: true,
        ScrollToBottom: true,
        ChatSearchBar: true,
        ChatApiDetailPanel: true,
        ChatImageFullscreenModal: true,
        ChatMessageBody: true,
        ChatMessageHeader: true,
        ChatSelectionPill: true,
        ChatToolGroup: true,
        ChatAvatar: true,
        ChatTimeDivider: true,
        SvgIcon: true,
      },
    },
  });
}

/** 走生产的取数路径（`toggleApiDetail` → `proxy_find_by_timestamp`）把详情装进面板 */
async function mountWithDetail() {
  const wrapper = mountChatView();
  await flushPromises();
  await (wrapper.vm as unknown as {
    toggleApiDetail: (index: number, timestamp: string) => Promise<void>;
  }).toggleApiDetail(0, TIMESTAMP);
  await flushPromises();
  // 切到响应体页签：提示只在响应侧渲染（请求侧仍是请求正文）
  (wrapper.vm as unknown as { apiDetailTab: "request" | "response" }).apiDetailTab = "response";
  await wrapper.vm.$nextTick();
  return wrapper;
}

function bodyHtml(wrapper: Awaited<ReturnType<typeof mountWithDetail>>): string {
  return (wrapper.vm as unknown as { apiBodyHtml: () => string }).apiBodyHtml();
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "proxy_status") return { enabled: false, running: false };
    if (command === "proxy_find_by_timestamp") return DETAIL;
    return undefined;
  });
  mocks.listeners.clear();
  vi.stubGlobal("crypto", { randomUUID: vi.fn(() => "request-id") });
  HTMLElement.prototype.scrollTo = vi.fn();
});

afterEach(() => setLocale("en"));

describe("聊天面板的上游失败提示按 error_kind 由前端渲染", () => {
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
      const html = bodyHtml(wrapper);
      // 句子随当前语言变
      expect(html, locale).toBe(expected[locale]);
      // 第三方原文逐字保留（R3：第三方文本永不翻译）
      expect(html, locale).toContain(THIRD_PARTY_TEXT);
    }
    wrapper.unmount();
  });

  it("阴性对照：error_kind 为 null 时不出现提示句，正文照常走渲染管线", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "proxy_status") return { enabled: false, running: false };
      if (command === "proxy_find_by_timestamp") return { ...DETAIL, error_kind: null };
      return undefined;
    });
    const wrapper = await mountWithDetail();

    // 同一个 res_body 在阳性对照里被包进提示句，这里必须落回正文管线——
    // 「恒显示提示句」与「只在 upstream 时显示」由此可区分。
    // `"{}"` 是 `highlightSseResponse` 对这条既非 JSON 也非 SSE 的串做聚合的结果：
    // 它证明正文确实进了那条管线，且提示句那一支排在管线**之前**。
    expect(bodyHtml(wrapper)).toBe("{}");
    expect(bodyHtml(wrapper)).not.toContain("Upstream request failed");
    wrapper.unmount();
  });

  it("阴性对照：error_kind 为未知值时同样不渲染提示句（兜底是原样展示正文）", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "proxy_status") return { enabled: false, running: false };
      if (command === "proxy_find_by_timestamp") return { ...DETAIL, error_kind: "timeout" };
      return undefined;
    });
    const wrapper = await mountWithDetail();

    expect(bodyHtml(wrapper)).toBe("{}");
    wrapper.unmount();
  });
});
