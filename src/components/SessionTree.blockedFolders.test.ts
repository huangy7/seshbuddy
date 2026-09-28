import { mount } from "@vue/test-utils";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import SessionTree from "./SessionTree.vue";
import type { AggregatedProjectInfo, SessionInfo } from "../types/session";

const blockedFoldersRef = ref<string[]>([]);

vi.mock("../composables/useBlockedFolders", () => ({
  useBlockedFolders: () => ({
    blockedFolders: blockedFoldersRef,
    isBlocked: (p: string) =>
      blockedFoldersRef.value.some((b) => p === b || p.startsWith(b + "/") || p.startsWith(b + "\\")),
    blockFolder: vi.fn(async (p: string) => {
      blockedFoldersRef.value.push(p);
    }),
    unblockFolder: vi.fn(),
  }),
}));

function makeSession(cliId: "claude", id: string): SessionInfo {
  return {
    session_id: id,
    file_path: `/workspace/${id}.jsonl`,
    display_name: `Session ${id}`,
    timestamp: "2026-08-28T10:00:00Z",
    file_size: 100,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

const mockProjects: AggregatedProjectInfo[] = [
  {
    encoded_dir: "proj-a",
    original_path: "/workspace/proj-a",
    project_key: "/workspace/proj-a",
    cli_ids: ["claude"],
    sessions: [makeSession("claude", "s1")],
  },
  {
    encoded_dir: "proj-b",
    original_path: "/workspace/secret/proj-b",
    project_key: "/workspace/secret/proj-b",
    cli_ids: ["claude"],
    sessions: [makeSession("claude", "s2")],
  },
  {
    encoded_dir: "proj-c",
    original_path: "/workspace/other/proj-c",
    project_key: "/workspace/other/proj-c",
    cli_ids: ["claude"],
    sessions: [makeSession("claude", "s3")],
  },
];

describe("SessionTree view layer blocked folders filtering", () => {
  beforeAll(() => {
    HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  it("filters out projects located within blocked folder paths", async () => {
    blockedFoldersRef.value = ["/workspace/secret"];

    const wrapper = mount(SessionTree, {
      props: {
        projects: mockProjects,
        selectedSessionIdentity: null,
        selectedSessionIdentities: [],
        searchQuery: "",
        isStreamingProjects: false,
        totalLoadedSessions: 3,
        isRefreshing: false,
        showLoadingIndicator: false,
        projectAllSelected: () => false,
        grouping: "directory",
      },
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const projectHeaders = wrapper.findAll(".project-header");
    // Should only have proj-a and proj-c (/workspace/secret/proj-b is filtered)
    expect(projectHeaders).toHaveLength(2);
    expect(wrapper.text()).toContain("proj-a");
    expect(wrapper.text()).toContain("proj-c");
    expect(wrapper.text()).not.toContain("proj-b");
  });
});
