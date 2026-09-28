import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import type { OpenTab } from "./useTabs";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => null),
}));

vi.mock("./useSessions", async () => {
  const { ref } = await import("vue");
  return {
    useSessions: () => ({
      projects: ref([
        {
          encoded_dir: "shared",
          original_path: "/workspace/shared",
          project_key: "/workspace/shared",
          cli_ids: ["claude", "codex"],
          sessions: [
            {
              session_id: "claude-session",
              file_path: "/sessions/shared.jsonl",
              cli_id: "claude",
            },
            {
              session_id: "codex-session",
              file_path: "/sessions/shared.jsonl",
              cli_id: "codex",
            },
          ],
        },
      ]),
    }),
  };
});

const { useSessionIdResolver } = await import("./useSessionIdResolver");

describe("useSessionIdResolver", () => {
  it("resolves equal file paths using each history tab's CLI identity", () => {
    const tabs = ref<OpenTab[]>([
      {
        id: "claude-tab",
        type: "history",
        label: { kind: "text", text: "Claude" },
        cliId: "claude",
        sessionPath: "/sessions/shared.jsonl",
      },
      {
        id: "codex-tab",
        type: "history",
        label: { kind: "text", text: "Codex" },
        cliId: "codex",
        sessionPath: "/sessions/shared.jsonl",
      },
    ]);
    const resolver = useSessionIdResolver(tabs);

    expect(resolver.historySessionId(tabs.value[0])).toBe("claude-session");
    expect(resolver.historySessionId(tabs.value[1])).toBe("codex-session");
  });
});
