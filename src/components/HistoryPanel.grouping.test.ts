import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import HistoryPanel from "./HistoryPanel.vue";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";
import type { ProjectInfo } from "../types/session";
import { t } from "../i18n";

const cliOptions: CliOption[] = (["claude", "codex"] as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: true,
  hasBinary: true,
}));

const mockProjects: ProjectInfo[] = [
  {
    encoded_dir: "shared",
    original_path: "/workspace/shared",
    sessions: [
      {
        session_id: "s1",
        file_path: "/workspace/shared/s1.jsonl",
        display_name: "Session 1",
        timestamp: "2026-08-28T10:00:00Z",
        file_size: 100,
        git_branch: "main",
        has_archive_snapshot: false,
        is_archived: false,
        cli_id: "claude",
      },
    ],
  },
];

describe("HistoryPanel tree grouping toggle", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => {
        store[k] = String(v);
      },
      removeItem: (k: string) => {
        delete store[k];
      },
      clear: () => {
        store = {};
      },
    });
  });

  function mountPanel(props: Record<string, unknown> = {}) {
    return mount(HistoryPanel, {
      props: {
        projects: mockProjects,
        selectedSessionIdentity: null,
        selectedSessionIdentities: [],
        searchQuery: "",
        sortMode: "time",
        isStreamingProjects: false,
        totalLoadedSessions: 1,
        isRefreshing: false,
        showLoadingIndicator: false,
        visibleCliIds: ["claude", "codex"],
        cliOptions,
        cliSessionCounts: { claude: 1, codex: 0 },
        projectAllSelected: () => false,
        ...props,
      },
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

  it("renders grouping ToggleGroup with role='radiogroup' and default directory selection", () => {
    const wrapper = mountPanel();
    const toggleGroup = wrapper.find(".tree-grouping-toggle");
    expect(toggleGroup.exists()).toBe(true);
    expect(toggleGroup.attributes("role")).toBe("radiogroup");

    const buttons = toggleGroup.findAll(".grouping-btn");
    expect(buttons).toHaveLength(2);

    const dirBtn = buttons[0];
    const providerBtn = buttons[1];

    expect(dirBtn.attributes("role")).toBe("radio");
    expect(dirBtn.attributes("aria-checked")).toBe("true");
    expect(dirBtn.classes()).toContain("active");

    expect(providerBtn.attributes("role")).toBe("radio");
    expect(providerBtn.attributes("aria-checked")).toBe("false");
    expect(providerBtn.classes()).not.toContain("active");
  });

  it("switches grouping to provider on click, persists to localStorage, and emits update:grouping", async () => {
    const wrapper = mountPanel();
    const buttons = wrapper.findAll(".grouping-btn");
    const providerBtn = buttons[1];

    await providerBtn.trigger("click");

    expect(providerBtn.attributes("aria-checked")).toBe("true");
    expect(providerBtn.classes()).toContain("active");
    expect(store["seshbuddy-tree-grouping"]).toBe("provider");
    expect(wrapper.emitted("update:grouping")?.[0]).toEqual(["provider"]);
  });

  it("initializes from localStorage preference", () => {
    store["seshbuddy-tree-grouping"] = "provider";
    const wrapper = mountPanel();
    const providerBtn = wrapper.findAll(".grouping-btn")[1];

    expect(providerBtn.attributes("aria-checked")).toBe("true");
    expect(providerBtn.classes()).toContain("active");
  });

  it("opens display settings from the locate control when the selected session is hidden", async () => {
    const wrapper = mountPanel({
      projects: [],
      selectedSessionIdentity: { cliId: "claude", filePath: "/archive/old.jsonl" },
      selectedSessionHiddenByVisibility: true,
    });
    const locateButton = wrapper.findAll(".tree-actions .toolbar-btn")[0];

    expect(locateButton.attributes("title")).toBe(t("session.historyPanel.locateHiddenByVisibility"));
    expect(locateButton.attributes("disabled")).toBeUndefined();
    await locateButton.trigger("click");
    expect(wrapper.emitted("openVisibilitySettings")).toHaveLength(1);
  });
});
