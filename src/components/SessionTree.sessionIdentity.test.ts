import { mount } from "@vue/test-utils";
import { beforeAll, describe, expect, it, vi } from "vitest";
import SessionTree from "./SessionTree.vue";
import type { AggregatedProjectInfo, SessionIdentity, SessionInfo } from "../types/session";

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

const projects: AggregatedProjectInfo[] = [{
  encoded_dir: "shared",
  original_path: "/workspace/shared",
  project_key: "/workspace/shared",
  cli_ids: ["claude", "codex"],
  sessions: [session("claude"), session("codex")],
}];

const claudeIdentity: SessionIdentity = { cliId: "claude", filePath: "/same/session.jsonl" };
const codexIdentity: SessionIdentity = { cliId: "codex", filePath: "/same/session.jsonl" };

function mountTree(selectedSessionIdentities: SessionIdentity[] = []) {
  return mount(SessionTree, {
    props: {
      projects,
      selectedSessionIdentity: codexIdentity,
      selectedSessionIdentities,
      contextMenuSessionIdentity: claudeIdentity,
      searchQuery: "",
      isStreamingProjects: false,
      totalLoadedSessions: 2,
      isRefreshing: false,
      showLoadingIndicator: false,
      projectAllSelected: () => false,
    },
    global: {
      stubs: { SvgIcon: true },
    },
  });
}

describe("SessionTree composite session identity", () => {
  beforeAll(() => {
    HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  it("distinguishes equal paths by CLI and emits the clicked identity", async () => {
    const wrapper = mountTree();
    await wrapper.vm.$nextTick();
    const rows = wrapper.findAll(".session-item");

    expect(rows).toHaveLength(2);
    expect(rows[0].classes()).toContain("context-target");
    expect(rows[0].classes()).not.toContain("selected");
    expect(rows[1].classes()).toContain("selected");

    await rows[1].trigger("click");
    expect(wrapper.emitted("selectSession")?.[0]).toEqual([codexIdentity, "shared"]);

    await rows[1].trigger("dblclick");
    expect(wrapper.emitted("pinSession")?.[0]).toEqual([codexIdentity, "shared"]);

    await rows[1].trigger("contextmenu");
    expect(wrapper.emitted("contextMenuSession")?.[0]?.[1]).toEqual(codexIdentity);
  });

  it("emits composite identities for toggle and range selection", async () => {
    const wrapper = mountTree([claudeIdentity]);
    await wrapper.vm.$nextTick();
    const rows = wrapper.findAll(".session-item");

    expect(rows[0].classes()).toContain("multi-selected");
    expect(rows[1].classes()).not.toContain("multi-selected");

    await rows[1].find(".session-checkbox").trigger("click");
    expect(wrapper.emitted("toggleSelect")?.[0]?.[0]).toEqual(codexIdentity);

    await rows[1].trigger("click", { shiftKey: true });
    expect(wrapper.emitted("rangeSelect")?.[0]).toEqual([claudeIdentity, codexIdentity]);
  });

  it("keeps collapse state per merged workspace and shows each row's source avatar", async () => {
    const wrapper = mount(SessionTree, {
      props: {
        projects: [
          {
            ...projects[0],
            encoded_dir: "shared-source",
            project_key: "/workspace/claude",
            sessions: [session("claude")],
            cli_ids: ["claude"],
          },
          {
            ...projects[0],
            encoded_dir: "shared-source",
            original_path: "/workspace/codex",
            project_key: "/workspace/codex",
            sessions: [session("codex")],
            cli_ids: ["codex"],
          },
        ],
        selectedSessionIdentity: null,
        selectedSessionIdentities: [],
        searchQuery: "",
        isStreamingProjects: false,
        totalLoadedSessions: 2,
        isRefreshing: false,
        showLoadingIndicator: false,
        projectAllSelected: () => false,
      },
      global: { stubs: { SvgIcon: true } },
    });

    await wrapper.findAll(".project-header")[1].trigger("click");

    expect(wrapper.findAll(".session-list")).toHaveLength(1);
    expect(wrapper.findAll(".session-cli-avatar")).toHaveLength(1);
    expect(wrapper.find(".session-cli-avatar").classes()).toContain("brand-codex");
  });
});
