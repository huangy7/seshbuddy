import { describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";
import type { ProjectInfo, SessionIdentity } from "../types/session";

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

const { mount } = await import("@vue/test-utils");
const HistoryPanel = (await import("./HistoryPanel.vue")).default;
const { t } = await import("../i18n");

const claudeIdentity: SessionIdentity = { cliId: "claude", filePath: "/claude.jsonl" };

const cliOptions: CliOption[] = (["claude", "codex"] as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: true,
  hasBinary: true,
}));

const allSourceProjects: ProjectInfo[] = [{
  encoded_dir: "shared",
  original_path: "/workspace/shared",
  sessions: [
    {
      session_id: "claude-session",
      file_path: "/claude.jsonl",
      display_name: "Claude 对话",
      timestamp: "2026-08-27T08:00:00Z",
      file_size: 1,
      git_branch: "main",
      has_archive_snapshot: false,
      is_archived: false,
      cli_id: "claude",
    },
    {
      session_id: "codex-session",
      file_path: "/codex.jsonl",
      display_name: "Codex 对话",
      timestamp: "2026-08-27T08:00:00Z",
      file_size: 1,
      git_branch: "main",
      has_archive_snapshot: false,
      is_archived: false,
      cli_id: "codex",
    },
  ],
}];

const claudeOnlyProjects: ProjectInfo[] = [{
  ...allSourceProjects[0],
  sessions: [allSourceProjects[0].sessions[0]],
}];

function mountPanel(
  visibleCliIds: CliId[],
  projects: ProjectInfo[] = allSourceProjects,
  cliSessionCounts: Partial<Record<CliId, number>> = { claude: 1, codex: 1 },
  scanCliErrors: Partial<Record<CliId, string>> = {},
  attachTo?: Element,
) {
  return mount(HistoryPanel, {
    props: {
      projects,
      selectedSessionIdentity: claudeIdentity,
      selectedSessionIdentities: [],
      searchQuery: "",
      sortMode: "time",
      isStreamingProjects: false,
      totalLoadedSessions: 2,
      isRefreshing: false,
      showLoadingIndicator: false,
      visibleCliIds,
      cliOptions,
      cliSessionCounts,
      scanCliErrors,
      projectAllSelected: () => false,
    },
    attachTo,
    global: {
      stubs: {
        SearchBar: true,
        SessionTree: true,
        SvgIcon: true,
        ChatAvatar: true,
      },
    },
  });
}

describe("HistoryPanel CLI filter", () => {
  it("renders as a pure conversation list without sub-tabs or bookmark entries", () => {
    const wrapper = mountPanel(["claude", "codex"]);

    expect(wrapper.find(".sub-tab").exists()).toBe(false);
    expect(wrapper.text()).not.toContain(t("session.favoritesPanel.title"));
  });

  it("emits filter state when a CLI is checked or unchecked without changing the current CLI", async () => {
    const wrapper = mountPanel(["claude"]);

    await wrapper.find(".cli-filter-trigger").trigger("click");
    await wrapper.find('[data-cli-id="codex"]').trigger("click");

    expect(wrapper.emitted("update:cliFilter")).toEqual([
      [{ mode: "custom", cliIds: ["claude", "codex"] }],
    ]);
    expect(wrapper.emitted("changeCli")).toBeUndefined();

    await wrapper.setProps({ visibleCliIds: ["claude", "codex"] });
    await wrapper.find('[data-cli-id="claude"]').trigger("click");

    expect(wrapper.emitted("update:cliFilter")).toEqual([
      [{ mode: "custom", cliIds: ["claude", "codex"] }],
      [{ mode: "custom", cliIds: ["codex"] }],
    ]);
    expect(wrapper.emitted("changeCli")).toBeUndefined();
  });

  it("toggles the all-sources checkbox between empty custom selection and all sources", async () => {
    const wrapper = mountPanel(["claude", "codex"]);

    await wrapper.find(".cli-filter-trigger").trigger("click");
    await wrapper.find(".cli-filter-all").trigger("click");

    expect(wrapper.emitted("update:cliFilter")).toEqual([
      [{ mode: "custom", cliIds: [] }],
    ]);

    await wrapper.setProps({ visibleCliIds: [] });
    await wrapper.find(".cli-filter-all").trigger("click");

    expect(wrapper.emitted("update:cliFilter")).toEqual([
      [{ mode: "custom", cliIds: [] }],
      [{ mode: "all" }],
    ]);

    await wrapper.setProps({ visibleCliIds: ["claude"] });
    await wrapper.find(".cli-filter-all").trigger("click");

    expect(wrapper.emitted("update:cliFilter")?.[2]).toEqual([{ mode: "all" }]);
  });

  it("keeps hidden-source counts from unfiltered source statistics", async () => {
    const wrapper = mountPanel(
      ["claude"],
      claudeOnlyProjects,
      { claude: 3, codex: 8 },
    );

    await wrapper.find(".cli-filter-trigger").trigger("click");

    expect(wrapper.find('[data-cli-id="codex"] .cli-filter-option-count').text()).toBe("8");
  });

  it("shows a warning row with the CLI name for failed sources and emits retryCli", async () => {
    const wrapper = mountPanel(
      ["claude", "codex"],
      allSourceProjects,
      { claude: 1, codex: 1 },
      { codex: "读取数据目录失败" },
    );

    await wrapper.find(".cli-filter-trigger").trigger("click");

    const errorRow = wrapper.find('[data-cli-error="codex"]');
    expect(errorRow.exists()).toBe(true);
    expect(errorRow.text()).toContain("Codex");
    expect(wrapper.find('[data-cli-error="claude"]').exists()).toBe(false);

    await errorRow.find(".cli-filter-retry").trigger("click");
    expect(wrapper.emitted("retryCli")).toEqual([["codex"]]);
  });
});

describe("HistoryPanel CLI filter menu keyboard contract", () => {
  function mountAttached(visibleCliIds: CliId[] = ["claude", "codex"]) {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const wrapper = mountPanel(visibleCliIds, allSourceProjects, { claude: 1, codex: 1 }, {}, host);
    return { wrapper, host };
  }

  it("opens with focus on the first item and cycles focus with arrow keys", async () => {
    const { wrapper, host } = mountAttached();
    await wrapper.find(".cli-filter-trigger").trigger("click");

    const items = () => Array.from(host.querySelectorAll<HTMLButtonElement>(".cli-filter-option"));
    expect(document.activeElement).toBe(items()[0]);

    await wrapper.find(".cli-filter-menu").trigger("keydown", { key: "ArrowDown" });
    expect(document.activeElement).toBe(items()[1]);

    await wrapper.find(".cli-filter-menu").trigger("keydown", { key: "ArrowUp" });
    expect(document.activeElement).toBe(items()[0]);

    await wrapper.find(".cli-filter-menu").trigger("keydown", { key: "ArrowUp" });
    expect(document.activeElement).toBe(items()[items().length - 1]);

    wrapper.unmount();
    host.remove();
  });

  it("closes on Escape and returns focus to the trigger", async () => {
    const { wrapper, host } = mountAttached();
    const trigger = wrapper.find(".cli-filter-trigger");
    await trigger.trigger("click");
    expect(wrapper.find(".cli-filter-menu").exists()).toBe(true);

    await wrapper.find(".cli-filter-menu").trigger("keydown", { key: "Escape" });

    expect(wrapper.find(".cli-filter-menu").exists()).toBe(false);
    expect(document.activeElement).toBe(trigger.element);

    wrapper.unmount();
    host.remove();
  });

  it("does not swallow Escape or arrow keys while the menu is closed", async () => {
    const { wrapper, host } = mountAttached();
    const seen: string[] = [];
    const onDocKeydown = (e: KeyboardEvent) => seen.push(e.key);
    document.addEventListener("keydown", onDocKeydown);

    for (const key of ["Escape", "ArrowDown"]) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      wrapper.find(".cli-filter-trigger").element.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
    expect(seen).toEqual(["Escape", "ArrowDown"]);

    document.removeEventListener("keydown", onDocKeydown);
    wrapper.unmount();
    host.remove();
  });

  it("closes when clicking outside the menu", async () => {
    const { wrapper, host } = mountAttached();
    await wrapper.find(".cli-filter-trigger").trigger("click");
    expect(wrapper.find(".cli-filter-menu").exists()).toBe(true);

    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();

    expect(wrapper.find(".cli-filter-menu").exists()).toBe(false);

    wrapper.unmount();
    host.remove();
  });
});
