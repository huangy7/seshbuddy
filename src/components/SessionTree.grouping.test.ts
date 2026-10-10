import { beforeAll, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import SessionTree from "./SessionTree.vue";
import type { AggregatedProjectInfo, SessionIdentity, SessionInfo } from "../types/session";
import { setLocale, t } from "../i18n";

function session(cliId: "claude" | "codex", id: string, time: string): SessionInfo {
  return {
    session_id: `${cliId}-${id}`,
    file_path: `/workspace/${cliId}/${id}.jsonl`,
    display_name: `${cliId === "claude" ? "Claude" : "Codex"} session ${id}`,
    timestamp: time,
    file_size: 100,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

const mockProjects: AggregatedProjectInfo[] = [
  {
    encoded_dir: "shared",
    original_path: "/workspace/shared",
    project_key: "/workspace/shared",
    cli_ids: ["claude", "codex"],
    sessions: [
      session("claude", "c1", "2026-08-28T10:00:00Z"),
      session("claude", "c2", "2026-08-28T09:00:00Z"),
      session("codex", "x1", "2026-08-28T08:00:00Z"),
    ],
  },
  {
    encoded_dir: "codex-only",
    original_path: "/workspace/codex-only",
    project_key: "/workspace/codex-only",
    cli_ids: ["codex"],
    sessions: [session("codex", "x2", "2026-08-28T11:00:00Z")],
  },
];

function mountTree(props: Record<string, unknown> = {}) {
  return mount(SessionTree, {
    props: {
      projects: mockProjects,
      selectedSessionIdentity: null,
      selectedSessionIdentities: [],
      searchQuery: "",
      isStreamingProjects: false,
      totalLoadedSessions: 4,
      isRefreshing: false,
      showLoadingIndicator: false,
      projectAllSelected: () => false,
      grouping: "directory",
      ...props,
    },
    global: {
      stubs: { SvgIcon: true },
    },
  });
}

describe("SessionTree grouping modes", () => {
  beforeAll(() => {
    HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  it("renders empty hint when there are no visible projects", () => {
    const wrapper = mountTree({ projects: [] });
    expect(wrapper.find(".empty-hint").text()).toBe(t("session.sessionTree.emptyNoVisibleSessions"));
  });

  it("offers display settings when the empty list contains sessions hidden by visibility", async () => {
    const wrapper = mountTree({ projects: [], hasSessionsHiddenByVisibility: true });
    const button = wrapper.get(".empty-hint button");
    expect(button.text()).toBe(t("session.sessionTree.adjustVisibilitySettings"));

    await button.trigger("click");
    expect(wrapper.emitted("openVisibilitySettings")).toHaveLength(1);
  });

  it("renders 2-level tree in directory mode", async () => {
    const wrapper = mountTree({ grouping: "directory" });

    // Level 1: Projects
    const projectHeaders = wrapper.findAll(".project-header");
    expect(projectHeaders).toHaveLength(2);
    expect(wrapper.findAll(".provider-node")).toHaveLength(0);

    // Expand first project
    await projectHeaders[0].trigger("click");
    const sessionItems = wrapper.findAll(".session-item");
    expect(sessionItems).toHaveLength(3); // 2 claude + 1 codex
  });

  it("renders 3-level tree in provider mode with provider headers, count capsules, and nested projects", async () => {
    const wrapper = mountTree({ grouping: "provider" });

    // Level 1: Provider nodes (Claude Code and Codex)
    const providerNodes = wrapper.findAll(".provider-node");
    expect(providerNodes).toHaveLength(2);

    // Provider headers have avatar and count capsule
    const providerHeaders = wrapper.findAll(".provider-header");
    expect(providerHeaders).toHaveLength(2);

    const claudeHeader = providerHeaders[0];
    expect(claudeHeader.find(".provider-name").text()).toBe("Claude Code");
    expect(claudeHeader.find(".provider-count-capsule").text()).toBe("2");

    const codexHeader = providerHeaders[1];
    expect(codexHeader.find(".provider-name").text()).toBe("Codex");
    expect(codexHeader.find(".provider-count-capsule").text()).toBe("2");

    // Level 2: By default provider nodes are expanded, projects inside are collapsed
    const nestedProjectHeaders = wrapper.findAll(".nested-project-header");
    // Claude has 1 project (/workspace/shared), Codex has 2 projects (/workspace/codex-only, /workspace/shared)
    expect(nestedProjectHeaders).toHaveLength(3);

    // Level 3: Expand Claude's project
    await nestedProjectHeaders[0].trigger("click");
    const claudeSessions = wrapper.findAll(".nested-session-list .session-item");
    expect(claudeSessions).toHaveLength(2);
    expect(claudeSessions[0].text()).toContain("Claude session c1");
    expect(claudeSessions[1].text()).toContain("Claude session c2");
  });

  it("maintains independent collapse states between directory mode and provider mode", async () => {
    const wrapper = mountTree({ grouping: "directory" });

    // In directory mode, expand /workspace/shared
    await wrapper.findAll(".project-header")[0].trigger("click");
    expect(wrapper.findAll(".session-item")).toHaveLength(3);

    // Switch to provider mode
    await wrapper.setProps({ grouping: "provider" });
    // Projects inside provider groups should still be in their own default collapsed state
    expect(wrapper.findAll(".session-item")).toHaveLength(0);

    // Expand /workspace/shared under Claude
    await wrapper.findAll(".nested-project-header")[0].trigger("click");
    expect(wrapper.findAll(".session-item")).toHaveLength(2);

    // Switch back to directory mode
    await wrapper.setProps({ grouping: "directory" });
    // Directory mode project should still be expanded!
    expect(wrapper.findAll(".session-item")).toHaveLength(3);
  });

  it("locates session in provider mode by opening both provider and nested project nodes", async () => {
    const targetIdentity: SessionIdentity = {
      cliId: "codex",
      filePath: "/workspace/codex/x1.jsonl",
    };

    const wrapper = mountTree({
      grouping: "provider",
      selectedSessionIdentity: null,
    });

    // Collapse Codex provider group first
    await wrapper.findAll(".provider-header")[1].trigger("click");
    expect(wrapper.findAll(".nested-project-header")).toHaveLength(1); // Only Claude projects visible

    // Set selectedSessionIdentity to Codex session
    await wrapper.setProps({ selectedSessionIdentity: targetIdentity });
    await wrapper.vm.$nextTick();

    // Now Codex provider node and nested project node should both be open
    const openSessions = wrapper.findAll(".session-item");
    expect(openSessions.length).toBeGreaterThan(0);
    const selectedItem = wrapper.find(".session-item.selected");
    expect(selectedItem.exists()).toBe(true);
    expect(selectedItem.text()).toContain("Codex session x1");
  });

  it("handles keyboard navigation across visible sessions in provider mode", async () => {
    const wrapper = mountTree({ grouping: "provider" });

    // Expand Claude's project
    await wrapper.findAll(".nested-project-header")[0].trigger("click");

    const tree = wrapper.find(".session-tree");
    await tree.trigger("keydown", { key: "ArrowDown" });

    expect(wrapper.emitted("selectSession")?.[0]).toEqual([
      { cliId: "claude", filePath: "/workspace/claude/c1.jsonl" },
      "shared",
    ]);
  });
});

/**
 * 项目路径缺席时的**渲染**证据（R4）。
 *
 * 后端不再造「未知项目」占位串：解析不出项目路径时 `original_path` 是 `null`，项目名位由
 * `lastPathSegment(null)` 按当前语言渲染兜底句。断言的是挂载后 DOM 上的文本，四语各切一次
 * 并等一次重渲染（不等就是恒真断言）。
 */
describe("SessionTree 项目路径缺席", () => {
  const absentProject: AggregatedProjectInfo[] = [
    {
      encoded_dir: null,
      original_path: null,
      project_key: "absent",
      cli_ids: ["claude"],
      sessions: [session("claude", "n1", "2026-08-28T10:00:00Z")],
    },
  ];

  it.each(["directory", "provider"] as const)(
    "项目名位渲染本地化兜底句（四语，%s 分组）",
    async (grouping) => {
      const wrapper = mountTree({ projects: absentProject, grouping, totalLoadedSessions: 1 });
      const rendered: string[] = [];
      const titles: (string | undefined)[] = [];
      for (const lang of ["en", "zh", "ja", "de"] as const) {
        setLocale(lang);
        await wrapper.vm.$nextTick();
        const node = wrapper.find(".project-path");
        expect(node.exists(), `${lang}: 项目名位应渲染出来`).toBe(true);
        rendered.push(node.text());
        titles.push(node.attributes("title"));
      }
      expect(rendered).toEqual([
        "Unknown project",
        "未知项目",
        "不明なプロジェクト",
        "Unbekanntes Projekt",
      ]);
      // 缺席时悬停提示回落到同一句兜底文案（不能显示字面量 null）
      expect(titles).toEqual(rendered);
      for (const text of rendered) expect(text).not.toContain("null");
    },
  );

  /**
   * 阳性对照：`title` 必须仍是**完整路径**。
   *
   * 可见文本一直是最后一段（`lastPathSegment`），`title` 是它唯一的补全途径；
   * 若把 `title` 也改成 `lastPathSegment(...)`，这一段信息就没了 —— 而且是在**路径存在**的
   * 常见情况下丢的，不只是缺席时。
   *
   * 按分组模式各跑一遍：provider 模式的 `title` 是**另一处**模板（`.nested-project-header`
   * 里的那一个），此前只有 directory 模式被覆盖 —— 那一处改回 `lastPathSegment(...)`
   * 套件仍然全绿。
   */
  it.each(["directory", "provider"] as const)(
    "项目路径存在时，悬停提示给出完整路径而不是最后一段（%s 分组）",
    (grouping) => {
      const wrapper = mountTree({ projects: mockProjects, grouping });
      const nodes = wrapper.findAll(".project-path");

      // 阳性对照：两种模式下都渲染出了多个项目名位——否则下面的循环可以一个节点都不看
      expect(nodes.length).toBeGreaterThanOrEqual(2);
      const pairs = nodes.map((node) => [node.text(), node.attributes("title")] as const);
      expect(pairs.map(([text]) => text).sort()).toEqual(
        grouping === "directory"
          ? ["codex-only", "shared"]
          : ["codex-only", "shared", "shared"],
      );
      for (const [text, title] of pairs) {
        expect(title).toBe(`/workspace/${text}`);
        expect(title).not.toBe(text);
      }
    },
  );
});
