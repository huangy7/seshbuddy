import { describe, expect, it, vi } from "vitest";
import type { CliOption } from "../types/cli";
import type { SessionInfo } from "../types/session";

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

const {
  aggregateProjectItems,
  groupSessionIdentities,
  resolveLaunchCliId,
  shouldRefreshForCli,
} = await import("./useSessions");

function session(overrides: Partial<SessionInfo>): SessionInfo {
  return {
    session_id: "session-id",
    file_path: "/sessions/default.jsonl",
    display_name: "Session",
    timestamp: "2026-08-27T08:00:00Z",
    file_size: 10,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: "claude",
    ...overrides,
  };
}

function cliOption(
  id: CliOption["id"],
  overrides: Partial<Pick<CliOption, "hasBinary" | "supportsNewSession">> = {},
): CliOption {
  return {
    id,
    name: id,
    command: id,
    dataSourcePath: "",
    dataDirPath: "",
    installHintKey: "",
    permissionLabelKey: "",
    permissionHintKey: "",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: false,
    supportsApiProfiles: false,
    supportsApiLogs: false,
    hasSessions: true,
    hasBinary: true,
    ...overrides,
  };
}

describe("multi-CLI session store helpers", () => {
  it("merges normalized project paths, retains source CLIs, and sorts sessions newest-first", () => {
    const projects = aggregateProjectItems([
      {
        encoded_dir: "claude-app",
        original_path: "/workspace/app/",
        session: session({
          cli_id: "claude",
          file_path: "/claude/older.jsonl",
          timestamp: "2026-08-26T08:00:00Z",
        }),
      },
      {
        encoded_dir: "codex-app",
        original_path: "/workspace/app",
        session: session({
          cli_id: "codex",
          file_path: "/codex/newer.jsonl",
          timestamp: "2026-08-27T08:00:00Z",
        }),
      },
    ]);

    expect(projects).toHaveLength(1);
    expect(projects[0]).toMatchObject({
      project_key: "/workspace/app",
      cli_ids: ["claude", "codex"],
    });
    expect(projects[0].sessions.map((item) => `${item.cli_id}:${item.file_path}`)).toEqual([
      "codex:/codex/newer.jsonl",
      "claude:/claude/older.jsonl",
    ]);
  });

  /**
   * 后端把「解析不出项目路径」序列化成 `null`（`session.rs` 的 `ProjectSessionChunkItem`，
   * 无 `skip_serializing_if`）。聚合层曾经假设它是 `string`，于是 `normalizeProjectPath(null)`
   * 当场 `TypeError: Cannot read properties of null (reading 'trim')` —— 项目列表整块渲染失败。
   *
   * 可达路径：`scan.rs` 的 `load_codex_projects_snapshot` / `load_antigravity_projects_snapshot`
   * 在会话没有 cwd 时给出 `original_path: None`，经 `scan_projects` 流式回传。
   */
  it("原样接受缺席的项目路径（null），不抛异常且与空串归入同一组", () => {
    const projects = aggregateProjectItems([
      {
        encoded_dir: null,
        original_path: null,
        session: session({ cli_id: "codex", file_path: "/codex/absent.jsonl" }),
      },
      {
        encoded_dir: null,
        original_path: null,
        session: session({ cli_id: "antigravity", file_path: "/ag/absent.jsonl" }),
      },
      {
        encoded_dir: "claude-app",
        original_path: "/workspace/app",
        session: session({ cli_id: "claude", file_path: "/claude/present.jsonl" }),
      },
    ]);

    expect(projects).toHaveLength(2);
    const absent = projects.find((project) => project.original_path === null);
    expect(absent, "缺席的项目必须仍然出现在列表里").toBeTruthy();
    expect(absent!.project_key).toBe("");
    expect(absent!.encoded_dir).toBe(null);
    expect(absent!.sessions).toHaveLength(2);
    expect(absent!.cli_ids).toEqual(["codex", "antigravity"]);

    // 阳性对照：真实路径仍走归一化，且不会与缺席组撞组
    const present = projects.find((project) => project.original_path === "/workspace/app");
    expect(present!.project_key).toBe("/workspace/app");
    expect(absent!.project_key).not.toBe(present!.project_key);
  });

  it("preserves POSIX and Windows roots while keeping UNC namespaces distinct", () => {
    const projects = aggregateProjectItems([
      { encoded_dir: "posix", original_path: "/", session: session({ file_path: "/root.jsonl" }) },
      { encoded_dir: "drive-a", original_path: "C:\\", session: session({ file_path: "C:\\a.jsonl" }) },
      { encoded_dir: "drive-b", original_path: "c:/", session: session({ cli_id: "codex", file_path: "C:\\b.jsonl" }) },
      { encoded_dir: "unc-a", original_path: "\\\\server\\share", session: session({ file_path: "//server/share/a.jsonl" }) },
      { encoded_dir: "local", original_path: "/server/share", session: session({ file_path: "/server/share/a.jsonl" }) },
    ]);

    expect(projects.map((project) => project.project_key)).toEqual([
      "/",
      "c:/",
      "//server/share",
      "/server/share",
    ]);
    expect(projects.map((project) => project.original_path)).toEqual([
      "/",
      "C:\\",
      "\\\\server\\share",
      "/server/share",
    ]);
  });

  it("does not refresh the visible list for an update from a hidden CLI", () => {
    expect(shouldRefreshForCli(["claude"], "codex")).toBe(false);
    expect(shouldRefreshForCli(["claude"], "claude")).toBe(true);
    expect(shouldRefreshForCli(["claude"], undefined)).toBe(true);
  });

  it("groups equal file paths independently by CLI", () => {
    expect(
      [...groupSessionIdentities([
        { cliId: "claude", filePath: "/same/session.jsonl" },
        { cliId: "codex", filePath: "/same/session.jsonl" },
        { cliId: "claude", filePath: "/other/session.jsonl" },
      ])],
    ).toEqual([
      ["claude", ["/same/session.jsonl", "/other/session.jsonl"]],
      ["codex", ["/same/session.jsonl"]],
    ]);
  });

  it("prefers a sole visible creatable CLI before the last successful launch", () => {
    const options = [cliOption("claude"), cliOption("codex")];
    expect(resolveLaunchCliId(["codex"], options, "claude")).toBe("codex");
  });

  it("falls back from the last successful launch to the first installed creatable CLI", () => {
    const options = [
      cliOption("claude"),
      cliOption("codex", { hasBinary: false }),
    ];
    expect(resolveLaunchCliId(["claude", "codex"], options, "codex")).toBe("claude");
  });

  it("marks in-place fork support correctly for Claude and WorkBuddy", async () => {
    const { resolveCliDefinition } = await import("../types/cli");
    expect(resolveCliDefinition("claude").supportsInPlaceFork).toBe(true);
    expect(resolveCliDefinition("workbuddy").supportsInPlaceFork).toBe(true);
    expect(resolveCliDefinition("codex").supportsInPlaceFork).toBe(false);
    expect(resolveCliDefinition("gemini").supportsInPlaceFork).toBe(false);
    expect(resolveCliDefinition("dsh").supportsInPlaceFork).toBe(false);
  });
});
