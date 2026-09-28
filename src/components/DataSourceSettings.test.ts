import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { ref } from "vue";
import DataSourceSettings from "./DataSourceSettings.vue";
import { t } from "../i18n";
import { CLI_DEFINITIONS, type CliId, type CliOption, type CliPathConfig } from "../types/cli";
import type { CliFilterState } from "../composables/cliFilter";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  message: vi.fn(),
}));

const mockCliOptions: CliOption[] = (Object.keys(CLI_DEFINITIONS) as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: id === "claude" || id === "codex",
  hasBinary: true,
}));

const mockCliPathConfigs: Record<CliId, CliPathConfig | null> = {
  claude: null,
  codex: null,
  gemini: null,
  workbuddy: null,
  dsh: null,
};

const mockSessionCounts: Partial<Record<CliId, number>> = {
  claude: 183,
  codex: 131,
  gemini: 3,
  workbuddy: 12,
  dsh: 0,
};

const mockVisibleCliIds = ref<CliId[]>(["claude", "codex", "gemini", "workbuddy"]);
const mockAvailableCliIds = ref<CliId[]>(["claude", "codex", "gemini", "workbuddy"]);
const mockCliFilter = ref<CliFilterState>({ mode: "all" });

vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({
    cliOptions: ref(mockCliOptions),
    cliPathConfigs: ref(mockCliPathConfigs),
    cliSessionCounts: ref(mockSessionCounts),
    cliFilter: mockCliFilter,
    visibleCliIds: mockVisibleCliIds,
    availableCliIds: mockAvailableCliIds,
    setCliFilter: vi.fn(),
    setCliDataDirOverride: vi.fn(),
    refresh: vi.fn(),
  }),
}));

describe("DataSourceSettings.vue", () => {
  it("renders list of all CLI data sources with session counts and switch", () => {
    const wrapper = mount(DataSourceSettings, {
      global: {
        stubs: {
          ChatAvatar: true,
          SvgIcon: true,
          ToggleSwitch: true,
        },
      },
    });
    expect(wrapper.text()).toContain("Claude Code");
    expect(wrapper.text()).toContain(t("settings.dataSource.sessionCount", { count: 183 }));
    expect(wrapper.text()).toContain("Codex");
    expect(wrapper.text()).toContain(t("settings.dataSource.sessionCount", { count: 131 }));
    expect(wrapper.text()).toContain("WorkBuddy");
    expect(wrapper.text()).toContain(t("settings.dataSource.sessionCount", { count: 12 }));
    expect(wrapper.text()).toContain(t("settings.dataSource.enabled"));
  });

  it("renders action buttons for opening and picking directory", () => {
    const wrapper = mount(DataSourceSettings, {
      global: {
        stubs: {
          ChatAvatar: true,
          SvgIcon: true,
          ToggleSwitch: true,
        },
      },
    });
    const buttons = wrapper.findAll("button.icon-action-btn");
    expect(buttons.length).toBeGreaterThan(0);
  });

  it("shows a selected CLI as enabled even when it has no sessions yet", () => {
    mockCliFilter.value = { mode: "custom", cliIds: ["dsh"] };
    try {
      const wrapper = mount(DataSourceSettings, {
        global: { stubs: { ChatAvatar: true, SvgIcon: true } },
      });
      const row = wrapper.findAll(".datasource-row").find((item) => item.text().includes("DSH"));
      expect(row?.find('[role="switch"]').attributes("aria-checked")).toBe("true");
    } finally {
      mockCliFilter.value = { mode: "all" };
    }
  });
});
