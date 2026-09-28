import { describe, it, expect, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const { mount, flushPromises } = await import("@vue/test-utils");
const DashboardView = (await import("./DashboardView.vue")).default;
const { t, setLocale } = await import("../i18n");

const kpiResponse = {
  kpi: {
    quotaLimit: 20,
    usedQuota: 4.82,
    totalCost: "$4.82",
    totalCostRaw: 4.82,
    cacheHitRate: 83.6,
    inputTokens: "1.0M",
    outputTokens: "0.2M",
    cachedTokens: "5.0M",
    cacheCreationInputTokens: "0.5M",
    costTrend: null,
    deptManagementMode: "none",
  },
  modelData: [],
  trendData: null,
  updatedAtStr: "10:30:00",
};

const recent = [
  {
    cliId: "claude",
    filePath: "/proj/a.jsonl",
    encodedDir: "-proj",
    projectPath: "/workspace/proj-a",
    title: "评审分支代码与去重设计对齐",
    timestamp: "2026-08-27T08:00:00Z",
  },
  {
    cliId: "codex",
    filePath: "/proj/b.jsonl",
    encodedDir: "-proj",
    projectPath: "/workspace/proj-b",
    title: "多 CLI 历史对话聚合方案",
    timestamp: "2026-08-26T08:00:00Z",
  },
];

function mountDashboard(props: Record<string, unknown> = {}) {
  return mount(DashboardView, {
    props: { recentConversations: recent, ...props },
    global: {
      stubs: {
        SvgIcon: true,
        ChatAvatar: true,
      },
    },
  });
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(null);
});

describe("DashboardView 双模式入口", () => {
  it("默认「问 SeshBuddy」模式，并提供「全局搜索」切换", async () => {
    const wrapper = mountDashboard();
    const modes = wrapper.findAll(".mode-btn");
    expect(modes).toHaveLength(2);
    expect(modes[0].text()).toContain(t("session.dashboard.modeAssistant"));
    expect(modes[1].text()).toContain(t("session.dashboard.modeSearch"));
    expect(modes[0].classes()).toContain("active");
  });

  it("问 SeshBuddy 模式提交发出 submitAssistant", async () => {
    const wrapper = mountDashboard();
    await wrapper.find(".prompt-input").setValue("帮我总结本周工作");
    await wrapper.find(".prompt-box").trigger("submit.prevent");
    expect(wrapper.emitted("submitAssistant")).toEqual([["帮我总结本周工作"]]);
    expect(wrapper.emitted("submitSearch")).toBeUndefined();
  });

  it("切换到全局搜索模式后提交发出 submitSearch", async () => {
    const wrapper = mountDashboard();
    const modes = wrapper.findAll(".mode-btn");
    await modes[1].trigger("click");
    expect(wrapper.emitted("openFullSearch")).toBeDefined();
    await wrapper.find(".prompt-input").setValue("登录报错");
    await wrapper.find(".prompt-box").trigger("submit.prevent");
    expect(wrapper.emitted("submitSearch")).toEqual([["登录报错"]]);
    expect(wrapper.emitted("submitAssistant")).toBeUndefined();
  });

  it("全局搜索模式下点击输入框发出 openFullSearch", async () => {
    const wrapper = mountDashboard();
    const modes = wrapper.findAll(".mode-btn");
    await modes[1].trigger("click");
    await wrapper.find(".prompt-input").trigger("click");
    expect(wrapper.emitted("openFullSearch")).toBeDefined();
  });

  it("提示区展示全局搜索快捷键", () => {
    const wrapper = mountDashboard();
    const hint = wrapper.find(".prompt-hint");
    expect(hint.exists()).toBe(true);
    expect(hint.text()).toMatch(/⌘ ⇧ F|Ctrl \+ Shift \+ F/);
    expect(hint.text()).toContain(t("session.dashboard.modeSearch"));
  });

  it("空输入不提交", async () => {
    const wrapper = mountDashboard();
    await wrapper.find(".prompt-box").trigger("submit.prevent");
    expect(wrapper.emitted("submitAssistant")).toBeUndefined();
    expect(wrapper.emitted("submitSearch")).toBeUndefined();
  });
});

describe("DashboardView 最近对话", async () => {
  it("渲染最近对话列表，点击条目发出带 cliId 的 openSession", async () => {
    const wrapper = mountDashboard();
    const rows = wrapper.findAll(".recent-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("评审分支代码与去重设计对齐");
    await rows[1].trigger("click");
    expect(wrapper.emitted("openSession")).toEqual([
      [{ cliId: "codex", filePath: "/proj/b.jsonl" }, "-proj"],
    ]);
  });

  it("无最近对话时隐藏列表区域", async () => {
    const wrapper = mountDashboard({ recentConversations: [] });
    expect(wrapper.find(".recent-section").exists()).toBe(false);
  });

  it("正确解析 Windows 风格反斜杠路径的项目标签", async () => {
    const wrapper = mountDashboard({
      recentConversations: [
        {
          cliId: "claude",
          filePath: "C:\\Users\\user\\test.jsonl",
          encodedDir: "test",
          projectPath: "C:\\Users\\user\\AppData\\Roaming\\com.seshbuddy.app\\data\\assistant\\workspace",
          title: "搜索提到「重构」的会话",
          timestamp: "2026-08-27T08:00:00Z",
        },
      ],
    });
    expect(wrapper.find(".recent-meta").text()).toBe("workspace");
  });
});

describe("DashboardView 内联搜索结果区", () => {
  const searchResults = [
    {
      session_id: "s1",
      file_path: "/proj/a.jsonl",
      display_name: "登录报错排查",
      project_path: "/workspace/proj-a",
      snippet: "…登录接口返回 500，堆栈指向鉴权中间件…",
      matched_field: null,
      match_count: 3,
      first_match_message_index: 12,
      cli_id: "claude",
    },
    {
      session_id: "s2",
      file_path: "/proj/b.jsonl",
      display_name: "多 CLI 聚合方案",
      project_path: "/workspace/proj-b",
      snippet: "…搜索范围沿用 visibleCliIds…",
      matched_field: null,
      match_count: 1,
      first_match_message_index: null,
      cli_id: "codex",
    },
  ];

  it("无搜索关键词时不渲染内联结果区", async () => {
    const wrapper = mountDashboard();
    expect(wrapper.find(".search-inline").exists()).toBe(false);
  });

  it("带 searchQuery 与结果时渲染 CLI 标识、工作区与摘要", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    const rows = wrapper.findAll(".search-result-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("登录报错排查");
    expect(rows[0].text()).toContain("proj-a");
    expect(rows[0].text()).toContain(t("session.dashboard.matchCount", { count: 3 }));
    expect(rows[0].text()).toContain("鉴权中间件");
  });

  it("点击结果行发出 openSearchResult 携带完整 SearchResult", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    await wrapper.findAll(".search-result-row")[1].trigger("click");
    expect(wrapper.emitted("openSearchResult")).toEqual([[searchResults[1]]]);
  });

  it("「在完整搜索页中查看」发出 openFullSearch 携带当前关键词", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    await wrapper.find(".search-full-link").trigger("click");
    expect(wrapper.emitted("openFullSearch")).toEqual([["登录报错"]]);
  });

  it("「清除」发出 clearSearch", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    await wrapper.find(".search-clear-btn").trigger("click");
    expect(wrapper.emitted("clearSearch")).toHaveLength(1);
  });

  it("加载中显示搜索中状态", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults: [], searchLoading: true });
    expect(wrapper.find(".search-inline-status").text()).toContain(t("session.dashboard.searching"));
  });

  it("空结果显示空态并保留查询词", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults: [] });
    const status = wrapper.find(".search-inline-status");
    expect(status.exists()).toBe(true);
    expect(status.text()).toContain(t("session.dashboard.searchEmpty"));
    expect(wrapper.find(".search-inline").text()).toContain("登录报错");
  });

  it("focusSearch() 切换到搜索模式并聚焦输入框", async () => {
    const wrapper = mount(DashboardView, {
      attachTo: document.body,
      props: { recentConversations: recent },
      global: { stubs: { SvgIcon: true, ChatAvatar: true } },
    });
    (wrapper.vm as unknown as { focusSearch: () => void }).focusSearch();
    await flushPromises();
    const modes = wrapper.findAll(".mode-btn");
    expect(modes[1].classes()).toContain("active");
    expect(modes[1].attributes("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(wrapper.find(".prompt-input").element);
    wrapper.unmount();
  });
});

/**
 * 命中字段标签的**渲染**证据（R4）。后端只回 `matched_field` 结构化字段名，
 * 标签文案由本组件按语言渲染；`zh` 渲染出来必须与改造前的用户可见串逐字相同。
 */
describe("DashboardView 命中字段标签", () => {
  const hitResults = [
    {
      session_id: "s-title",
      file_path: "/proj/a.jsonl",
      display_name: "优化会话加载性能",
      project_path: "/workspace/proj-a",
      snippet: "优化会话加载性能",
      matched_field: "title",
      match_count: 1,
      first_match_message_index: 0,
      cli_id: "claude",
    },
    {
      session_id: "s-fm",
      file_path: "/proj/b.jsonl",
      display_name: "登录报错排查",
      project_path: "/workspace/proj-b",
      snippet: "帮我看看登录接口",
      matched_field: "first_message",
      match_count: 1,
      first_match_message_index: 0,
      cli_id: "codex",
    },
    {
      session_id: "s-content",
      file_path: "/proj/c.jsonl",
      display_name: "正文命中",
      project_path: "/workspace/proj-c",
      snippet: "正文里提到了优化",
      matched_field: null,
      match_count: 1,
      first_match_message_index: 0,
      cli_id: "claude",
    },
    {
      session_id: "s-sid",
      file_path: "/proj/d.jsonl",
      display_name: "会话 ID 命中",
      project_path: "/workspace/proj-d",
      snippet: "abc123-session",
      matched_field: "session_id",
      match_count: 1,
      first_match_message_index: 0,
      cli_id: "claude",
    },
    {
      session_id: "s-body",
      file_path: "/proj/e.jsonl",
      display_name: "正文命中（结构化）",
      project_path: "/workspace/proj-e",
      snippet: "正文里提到了优化",
      matched_field: "content",
      match_count: 1,
      first_match_message_index: 0,
      cli_id: "claude",
    },
  ];

  it("标题/首条消息/会话 ID 命中渲染本地化标签，正文命中不加标签（四语）", async () => {
    const wrapper = mountDashboard({ searchQuery: "优化", searchResults: hitResults });
    const rendered: string[] = [];
    for (const lang of ["en", "zh", "ja", "de"] as const) {
      setLocale(lang);
      await wrapper.vm.$nextTick();
      rendered.push(...wrapper.findAll(".search-result-snippet").map((node) => node.text()));
    }
    wrapper.unmount();
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
  });
});


/**
 * 项目路径缺席时的**渲染**证据（R4）。
 *
 * 后端不再造「未知项目」占位串：解析不出项目路径时 `project_path` / `projectPath` 是 `null`
 * （见 `src-tauri/src/session.rs` 的 `resolve_project_path`）。断言的是挂载后 DOM 上的文本，
 * 不是「函数应该会拼」；四语各切一次并等一次重渲染（不等就是恒真断言）。
 */
describe("DashboardView 项目路径缺席", () => {
  const absentRecent = [
    {
      cliId: "claude",
      filePath: "/proj/none.jsonl",
      encodedDir: "",
      projectPath: null,
      title: "没有项目路径的会话",
      timestamp: "2026-08-27T08:00:00Z",
    },
  ];
  const absentSearch = [
    {
      session_id: "s-none",
      file_path: "/proj/none.jsonl",
      display_name: "没有项目路径的会话",
      project_path: null,
      snippet: "命中片段",
      matched_field: null,
      match_count: 1,
      first_match_message_index: 0,
      cli_id: "claude",
    },
  ];

  async function renderedInEveryLanguage(selector: string): Promise<string[]> {
    const wrapper = mountDashboard({
      searchQuery: "命中",
      searchResults: absentSearch,
      recentConversations: absentRecent,
    });
    const out: string[] = [];
    for (const lang of ["en", "zh", "ja", "de"] as const) {
      setLocale(lang);
      await wrapper.vm.$nextTick();
      const node = wrapper.find(selector);
      expect(node.exists(), `${lang}: ${selector} 应渲染出来`).toBe(true);
      out.push(node.text());
    }
    wrapper.unmount();
    return out;
  }

  it("搜索结果的项目路径位渲染本地化兜底句", async () => {
    const rendered = await renderedInEveryLanguage(".search-result-meta");
    // 整串逐字写死：这是**渲染出来的**字符串（含路径位与它右边的匹配计数）
    expect(rendered).toEqual([
      "Unknown project · Matches: 1",
      "未知项目 · 1 处匹配",
      "不明なプロジェクト · 1 件一致",
      "Unbekanntes Projekt · Treffer: 1",
    ]);
    // 阳性对照：缺席没有渲染成字面量 null（`projectLabel(null)` 若写成 `?? projectPath` 会这样）
    for (const text of rendered) expect(text).not.toContain("null");
  });

  it("最近对话的项目路径位渲染本地化兜底句", async () => {
    const rendered = await renderedInEveryLanguage(".recent-meta");
    expect(rendered).toEqual([
      "Unknown project",
      "未知项目",
      "不明なプロジェクト",
      "Unbekanntes Projekt",
    ]);
  });

  it("分项搜索错误按【当前语言】渲染（存的是结构化错误，不是渲染结果）", async () => {
    const wrapper = mountDashboard({
      searchQuery: "命中",
      searchCliErrors: { claude: { code: "session.load_failed", params: { path: "/p.jsonl" } } },
    });

    const out: string[] = [];
    try {
      for (const lang of ["en", "zh", "ja", "de"] as const) {
        setLocale(lang);
        await wrapper.vm.$nextTick();
        const node = wrapper.find(".search-cli-error-hint");
        expect(node.exists(), `${lang}: 错误提示应渲染出来`).toBe(true);
        out.push(node.text());
      }
    } finally {
      setLocale("en");
      wrapper.unmount();
    }

    // 判据取「四语各不相同」：渲染若发生在**存储**那一刻（旧写法），四条会全是同一个语言，
    // 而单看某一语的对错分辨不出来。
    expect(new Set(out).size).toBe(4);
    expect(out[0]).toContain("Failed to load the session: /p.jsonl");
    expect(out[1]).toContain("无法加载会话: /p.jsonl");
  });
});
