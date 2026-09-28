import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  TREE_GROUPING_STORAGE_KEY,
  groupProjectsByProvider,
  useTreeGrouping,
  type TreeGrouping,
} from "./useTreeGrouping";
import type { AggregatedProjectInfo, SessionInfo } from "../types/session";

function createSession(
  cliId: string,
  sessionId: string,
  timestamp: string,
  filePath = `/path/${cliId}/${sessionId}.jsonl`,
): SessionInfo {
  return {
    session_id: sessionId,
    file_path: filePath,
    display_name: `${cliId} - ${sessionId}`,
    timestamp,
    file_size: 1024,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

describe("groupProjectsByProvider", () => {
  it("returns empty array when input is empty", () => {
    expect(groupProjectsByProvider([])).toEqual([]);
  });

  it("provider 模式按 cli_id 拆分聚合项目", () => {
    const mergedProject: AggregatedProjectInfo = {
      project_key: "/workspace/shared",
      encoded_dir: "shared",
      original_path: "/workspace/shared",
      cli_ids: ["claude", "codex"],
      sessions: [
        createSession("claude", "c1", "2026-08-28T10:00:00Z"),
        createSession("codex", "x1", "2026-08-28T09:00:00Z"),
        createSession("claude", "c2", "2026-08-28T08:00:00Z"),
      ],
    };

    const groups = groupProjectsByProvider([mergedProject]);

    expect(groups).toHaveLength(2);

    const claudeGroup = groups.find((g) => g.cliId === "claude");
    expect(claudeGroup).toBeDefined();
    expect(claudeGroup?.count).toBe(2);
    expect(claudeGroup?.projects).toHaveLength(1);
    expect(claudeGroup?.projects[0].project_key).toBe("/workspace/shared");
    expect(claudeGroup?.projects[0].cli_ids).toEqual(["claude"]);
    expect(claudeGroup?.projects[0].sessions).toHaveLength(2);
    expect(claudeGroup?.projects[0].sessions.map((s) => s.session_id)).toEqual(["c1", "c2"]);

    const codexGroup = groups.find((g) => g.cliId === "codex");
    expect(codexGroup).toBeDefined();
    expect(codexGroup?.count).toBe(1);
    expect(codexGroup?.projects).toHaveLength(1);
    expect(codexGroup?.projects[0].project_key).toBe("/workspace/shared");
    expect(codexGroup?.projects[0].cli_ids).toEqual(["codex"]);
    expect(codexGroup?.projects[0].sessions).toHaveLength(1);
    expect(codexGroup?.projects[0].sessions[0].session_id).toBe("x1");
  });

  it("组间按 cliOptions/SUPPORTED_CLIS 顺序排列，且过滤没有会话的 provider", () => {
    const projectGemini: AggregatedProjectInfo = {
      project_key: "/workspace/gemini-app",
      encoded_dir: "gemini-app",
      original_path: "/workspace/gemini-app",
      cli_ids: ["gemini"],
      sessions: [createSession("gemini", "g1", "2026-08-28T12:00:00Z")],
    };
    const projectClaude: AggregatedProjectInfo = {
      project_key: "/workspace/claude-app",
      encoded_dir: "claude-app",
      original_path: "/workspace/claude-app",
      cli_ids: ["claude"],
      sessions: [createSession("claude", "c1", "2026-08-28T07:00:00Z")],
    };

    // Even if passed in reverse order, groups should follow CLI order (claude before gemini)
    const groups = groupProjectsByProvider([projectGemini, projectClaude]);
    expect(groups.map((g) => g.cliId)).toEqual(["claude", "gemini"]);
  });

  it("同 CLI 内多个项目按其最新会话时间倒序排列", () => {
    const projectOlder: AggregatedProjectInfo = {
      project_key: "/workspace/older",
      encoded_dir: "older",
      original_path: "/workspace/older",
      cli_ids: ["claude"],
      sessions: [
        createSession("claude", "c-old-1", "2026-08-20T10:00:00Z"),
      ],
    };
    const projectNewer: AggregatedProjectInfo = {
      project_key: "/workspace/newer",
      encoded_dir: "newer",
      original_path: "/workspace/newer",
      cli_ids: ["claude"],
      sessions: [
        createSession("claude", "c-new-1", "2026-08-28T10:00:00Z"),
        createSession("claude", "c-new-2", "2026-08-25T10:00:00Z"),
      ],
    };

    const groups = groupProjectsByProvider([projectOlder, projectNewer]);
    expect(groups).toHaveLength(1);
    expect(groups[0].projects.map((p) => p.project_key)).toEqual([
      "/workspace/newer",
      "/workspace/older",
    ]);
  });
});

describe("useTreeGrouping", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, val: string) => {
        store[key] = String(val);
      },
      removeItem: (key: string) => {
        delete store[key];
      },
      clear: () => {
        store = {};
      },
    });
  });

  it("defaults to 'directory' when localStorage is empty", () => {
    const { grouping } = useTreeGrouping();
    expect(grouping.value).toBe("directory");
  });

  it("initializes from localStorage if valid", () => {
    store[TREE_GROUPING_STORAGE_KEY] = "provider";
    const { grouping } = useTreeGrouping();
    expect(grouping.value).toBe("provider");
  });

  it("ignores invalid values in localStorage and falls back to 'directory'", () => {
    store[TREE_GROUPING_STORAGE_KEY] = "invalid_mode";
    const { grouping } = useTreeGrouping();
    expect(grouping.value).toBe("directory");
  });

  it("setGrouping updates grouping and persists to localStorage", () => {
    const { grouping, setGrouping } = useTreeGrouping();
    expect(grouping.value).toBe("directory");

    setGrouping("provider");
    expect(grouping.value).toBe("provider");
    expect(store[TREE_GROUPING_STORAGE_KEY]).toBe("provider");

    setGrouping("directory");
    expect(grouping.value).toBe("directory");
    expect(store[TREE_GROUPING_STORAGE_KEY]).toBe("directory");
  });

  it("toggleGrouping switches between 'directory' and 'provider'", () => {
    const { grouping, toggleGrouping } = useTreeGrouping();
    expect(grouping.value).toBe("directory");

    toggleGrouping();
    expect(grouping.value).toBe("provider");
    expect(store[TREE_GROUPING_STORAGE_KEY]).toBe("provider");

    toggleGrouping();
    expect(grouping.value).toBe("directory");
    expect(store[TREE_GROUPING_STORAGE_KEY]).toBe("directory");
  });
});
