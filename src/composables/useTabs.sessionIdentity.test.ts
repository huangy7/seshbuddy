import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionIdentity, SessionInfo } from "../types/session";

const mocks = vi.hoisted(() => ({
  selectedSessionIdentities: undefined as ReturnType<typeof ref<SessionIdentity[]>> | undefined,
  activeSessionIdentity: undefined as ReturnType<typeof ref<SessionIdentity | null>> | undefined,
  batchDeleteSessions: vi.fn(async () => true),
}));

function session(cliId: "claude" | "codex"): SessionInfo {
  return {
    session_id: `${cliId}-session`,
    file_path: "/same/session.jsonl",
    display_name: `${cliId} session`,
    timestamp: "2026-08-27T08:00:00Z",
    file_size: 1,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

vi.mock("./useSessions", async () => {
  const { ref } = await import("vue");
  mocks.selectedSessionIdentities = ref<SessionIdentity[]>([]);
  mocks.activeSessionIdentity = ref<SessionIdentity | null>(null);
  return {
    useSessions: () => ({
      projects: ref([
        {
          encoded_dir: "shared",
          original_path: "/workspace/shared",
          project_key: "/workspace/shared",
          cli_ids: ["claude", "codex"],
          sessions: [session("claude"), session("codex")],
        },
      ]),
      selectedSessionPath: ref<string | null>(null),
      selectedProjectDir: ref<string | null>(null),
      clearActiveSession: vi.fn(),
      activeSessionIdentity: mocks.activeSessionIdentity,
      setActiveSession: vi.fn((identity: SessionIdentity) => {
        mocks.activeSessionIdentity!.value = identity;
      }),
      selectedSessionIdentities: mocks.selectedSessionIdentities,
      batchDeleteSessions: mocks.batchDeleteSessions,
    }),
  };
});
vi.mock("./usePtySession", () => ({
  usePtySession: () => ({ sessions: ref([]), closeSession: vi.fn() }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(async () => false) }));

const { useTabs } = await import("./useTabs");

describe("history tab composite identity", () => {
  beforeEach(() => {
    mocks.batchDeleteSessions.mockClear();
    mocks.selectedSessionIdentities!.value = [];
  });

  it("opens and resolves two pinned tabs for the same path under different CLIs", () => {
    const tabs = useTabs();
    tabs.openHistoryTab("/same/session.jsonl", "shared", { pinned: true, cliId: "claude" });
    tabs.openHistoryTab("/same/session.jsonl", "shared", { pinned: true, cliId: "codex" });

    expect(tabs.historyTabs.value.map((tab) => tab.cliId)).toEqual(["claude", "codex"]);
    expect(tabs.findSessionByIdentity({ cliId: "claude", filePath: "/same/session.jsonl" })?.session.session_id)
      .toBe("claude-session");
    expect(tabs.findSessionByIdentity({ cliId: "codex", filePath: "/same/session.jsonl" })?.session.session_id)
      .toBe("codex-session");
  });

  it("closes only the tab whose composite identity was deleted", async () => {
    const tabs = useTabs();
    tabs.openHistoryTab("/same/session.jsonl", "shared", { pinned: true, cliId: "claude" });
    tabs.openHistoryTab("/same/session.jsonl", "shared", { pinned: true, cliId: "codex" });
    mocks.selectedSessionIdentities!.value = [
      { cliId: "codex", filePath: "/same/session.jsonl" },
    ];

    await tabs.batchDeleteAndCloseTabs();

    expect(tabs.historyTabs.value).toHaveLength(1);
    expect(tabs.historyTabs.value[0].cliId).toBe("claude");
  });
});
