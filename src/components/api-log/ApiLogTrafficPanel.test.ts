import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "../../i18n";
import type { SessionTrafficSummary } from "../../composables/useProxy";

// 本用例只看侧栏标签，报文查看器不进断言；Monaco 在 jsdom 里加载即抛
// （`document.queryCommandSupported is not a function`），把它的入口挡在模块图外。
vi.mock("../../monaco-workers", () => ({ monaco: { editor: {} } }));

// 主题模块在 import 时就注册跨窗口主题广播：没有 Tauri IPC 时它会抛成
// unhandled rejection（与断言无关，但会让本次运行被标成「有未处理错误」）。
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));

// 组件在模块级读 localStorage（侧栏宽度记忆）：Node 自带的那个全局没有 getItem 方法，
// 会先于 jsdom 的实现被取到，这里换成一个内存实现。
vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

// 动态 import：上面两条 stub 必须在组件模块**求值之前**生效
const ApiLogTrafficPanel = (await import("./ApiLogTrafficPanel.vue")).default;

/**
 * 会话标签的**渲染**证据：后端只回结构化数据（`TrafficSessionLabel`），
 * 句子在前端按当前语言拼。断言的是挂载后 DOM 里的文本，不是「函数应该会拼」。
 */

const SID = "3f2a1b9c-4d5e-6f70-8192-a3b4c5d6e7f8";

function session(overrides: Partial<SessionTrafficSummary> = {}): SessionTrafficSummary {
  return {
    session_id: SID,
    display_name: { kind: "session", session_id: SID },
    project_path: "/tmp/project",
    request_count: 3,
    total_req_size: 1,
    total_res_size: 2,
    total_duration_ms: 3,
    first_timestamp: "2026-09-25T00:00:00Z",
    last_timestamp: "2026-09-25T00:00:01Z",
    ok_count: 3,
    error_count: 0,
    ...overrides,
  };
}

function mountPanel(sessions: SessionTrafficSummary[], expandedSessionId: string | null = null) {
  return mount(ApiLogTrafficPanel, {
    props: {
      sessions,
      sessionsTotal: sessions.length,
      expandedSessionId,
      expandedSessionTraffic: [],
      expandedSessionTotal: 0,
      expandedId: null,
      expandedDetail: null,
      detailLoading: false,
      detailTab: "response" as const,
      detailSection: "body" as const,
      detailBodyText: "",
      detailBodyLanguage: "plaintext",
      detailHeadersText: "",
      detailHeadersLanguage: "plaintext",
      detailHasTools: false,
      detailHasSkills: false,
      detailIsSse: false,
      copyState: "idle" as const,
      sessionLoading: false,
      loading: false,
      sessionSearch: "",
    },
  });
}

afterEach(() => setLocale("en"));

describe("会话标签由前端按语言渲染", () => {
  it("列表里的会话卡片：无标题时渲染本地化标签 + 截断 id", async () => {
    const wrapper = mountPanel([session()]);
    expect(wrapper.find(".session-entry-title").text()).toBe("Session 3f2a1b9c");
    expect(wrapper.find(".session-entry-title").attributes("title")).toBe("Session 3f2a1b9c");

    setLocale("zh");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".session-entry-title").text()).toBe("会话 3f2a1b9c");

    setLocale("ja");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".session-entry-title").text()).toBe("セッション 3f2a1b9c");

    setLocale("de");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".session-entry-title").text()).toBe("Sitzung 3f2a1b9c");
  });

  it("解析出标题的会话原样显示（是用户数据不是文案，不随语言变）", async () => {
    const wrapper = mountPanel([
      session({ display_name: { kind: "named", title: "优化会话加载性能" } }),
    ]);
    expect(wrapper.find(".session-entry-title").text()).toBe("优化会话加载性能");
    setLocale("de");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".session-entry-title").text()).toBe("优化会话加载性能");
  });

  it("未关联会话桶渲染本地化标签", async () => {
    const wrapper = mountPanel([
      session({ session_id: null, display_name: { kind: "ungrouped" }, project_path: "" }),
    ]);
    expect(wrapper.find(".session-entry-title").text()).toBe("Unlinked session");
    setLocale("zh");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".session-entry-title").text()).toBe("未关联会话");
  });

  it("进入会话后的面包屑标题与列表用同一条渲染路径", async () => {
    const wrapper = mountPanel([session()], SID);
    expect(wrapper.find(".session-nav-title").text()).toBe("Session 3f2a1b9c");
    setLocale("de");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".session-nav-title").text()).toBe("Sitzung 3f2a1b9c");
  });

  it("会话不在已加载列表里时仍走原来的兜底文案（行为未变）", async () => {
    const wrapper = mountPanel([], SID);
    expect(wrapper.find(".session-nav-title").text()).toBe("Session requests");
    setLocale("de");
    await wrapper.vm.$nextTick();
    // 兜底键的取值来自语言包，与标签是两回事：切到德语后它跟着语言包变成德语，
    // 但**用的仍是 `sessionFallbackName` 这个键**，不是 `sessionLabel` 的标签渲染。
    expect(wrapper.find(".session-nav-title").text()).toBe("Sitzungsanfragen");
  });
});
