import { describe, expect, it, vi } from "vitest";
import type { SearchResult } from "../types/session";

/**
 * 项目路径缺席时的**渲染**证据（R4）。
 *
 * 后端不再造「未知项目」占位串：解析不出项目路径时 `project_path` 是 `null`，
 * 结果行的路径位由本组件按当前语言渲染兜底句。断言的是挂载后 DOM 上的文本，
 * 四语各切一次并等一次重渲染（不等就是恒真断言）。
 */

const mocks = vi.hoisted(() => ({
  results: undefined as ReturnType<typeof import("vue").ref<SearchResult[]>> | undefined,
}));

vi.mock("../composables/useSessions", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.results = ref<SearchResult[]>([]);
  return {
    useSessions: () => ({
      globalSearch: vi.fn(),
      globalSearchResults: mocks.results,
      globalSearchLoading: ref(false),
      searchIndexProgress: ref(null),
      searchIndexBuildRequest: ref(null),
      searchIndexBuildSkipped: ref(false),
      startSearchIndexBuild: vi.fn(),
      dismissSearchIndexBuild: vi.fn(),
      resetSearchIndexBuildState: vi.fn(),
      searchPendingCliIds: ref([]),
      searchStaleCliIds: ref([]),
      searchCliErrors: ref({}),
      buildingCliIds: ref([]),
      buildPendingSearchIndex: vi.fn(),
      buildAllPendingSearchIndexes: vi.fn(),
    }),
  };
});

const { mount } = await import("@vue/test-utils");
const GlobalSearch = (await import("./GlobalSearch.vue")).default;
const { setLocale } = await import("../i18n");

const absentResult: SearchResult = {
  session_id: "s-none",
  file_path: "/proj/none.jsonl",
  display_name: "没有项目路径的会话",
  project_path: null,
  snippet: "命中片段",
  matched_field: null,
  match_count: 1,
  first_match_message_index: 0,
  cli_id: "claude",
};

describe("GlobalSearch 项目路径缺席", () => {
  it("结果行的路径位渲染本地化兜底句（四语）", async () => {
    mocks.results!.value = [absentResult];
    const wrapper = mount(GlobalSearch, {
      props: { initialQuery: "命中" },
      global: { stubs: { SvgIcon: true } },
    });

    const rendered: string[] = [];
    for (const lang of ["en", "zh", "ja", "de"] as const) {
      setLocale(lang);
      await wrapper.vm.$nextTick();
      const node = wrapper.find(".result-path");
      expect(node.exists(), `${lang}: 结果行路径位应渲染出来`).toBe(true);
      rendered.push(node.text());
    }
    expect(rendered).toEqual([
      "Unknown project",
      "未知项目",
      "不明なプロジェクト",
      "Unbekanntes Projekt",
    ]);
    for (const text of rendered) expect(text).not.toContain("null");
  });
});

/**
 * 命中字段标签的**渲染**证据（R4）。
 *
 * 后端不再把 `会话标题: ` / `首条消息: ` 前缀写进 `snippet`，改回结构化的
 * `matched_field`；标签由本组件按当前语言渲染。`zh` 渲染出来必须与改造前的
 * 用户可见串逐字相同（`会话标题: 优化会话加载性能`），否则就是顺带改了中文。
 */
describe("GlobalSearch 命中字段标签", () => {
  const results = [
    { ...absentResult, session_id: "s-title", snippet: "优化会话加载性能", matched_field: "title" as const },
    { ...absentResult, session_id: "s-fm", snippet: "帮我看看登录接口", matched_field: "first_message" as const },
    { ...absentResult, session_id: "s-content", snippet: "正文里提到了优化", matched_field: null },
    { ...absentResult, session_id: "s-sid", snippet: "abc123-session", matched_field: "session_id" as const },
    { ...absentResult, session_id: "s-body", snippet: "正文里提到了优化", matched_field: "content" as const },
  ];

  async function renderedSnippets(): Promise<string[]> {
    mocks.results!.value = results;
    const wrapper = mount(GlobalSearch, {
      props: { initialQuery: "优化" },
      global: { stubs: { SvgIcon: true } },
    });
    const out: string[] = [];
    for (const lang of ["en", "zh", "ja", "de"] as const) {
      setLocale(lang);
      await wrapper.vm.$nextTick();
      out.push(...wrapper.findAll(".result-snippet").map((node) => node.text()));
    }
    wrapper.unmount();
    return out;
  }

  it("标题/首条消息/会话 ID 命中渲染本地化标签，正文命中不加标签（四语）", async () => {
    const rendered = await renderedSnippets();
    // 五行结果 × 四语，逐字写死渲染出来的字符串
    expect(rendered).toEqual([
      "Title: 优化会话加载性能",
      "First message: 帮我看看登录接口",
      "正文里提到了优化",
      "Session ID: abc123-session",
      "正文里提到了优化",
      "会话标题: 优化会话加载性能",
      "首条消息: 帮我看看登录接口",
      "正文里提到了优化",
      "会话 ID: abc123-session",
      "正文里提到了优化",
      "タイトル: 优化会话加载性能",
      "最初のメッセージ: 帮我看看登录接口",
      "正文里提到了优化",
      "セッション ID: abc123-session",
      "正文里提到了优化",
      "Titel: 优化会话加载性能",
      "Erste Nachricht: 帮我看看登录接口",
      "正文里提到了优化",
      "Sitzungs-ID: abc123-session",
      "正文里提到了优化",
    ]);
    // 阳性对照：标签不是从后端串里剥出来的——后端今天根本不发这两个词
    for (const text of rendered) expect(text).not.toContain("undefined");
  });
});
