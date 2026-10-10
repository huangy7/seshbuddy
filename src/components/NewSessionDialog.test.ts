import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import NewSessionDialog from "./NewSessionDialog.vue";

const invokeAppMock = vi.fn();
vi.mock("../utils/invokeApp", () => ({
  invokeApp: (cmd: string, args: any) => invokeAppMock(cmd, args),
}));

vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({
    projects: { value: [{ original_path: "/workspace/proj1" }, { original_path: "/workspace/proj2" }] },
    launchCliId: { value: "claude" },
    skipPermissions: { value: false },
  }),
}));

vi.mock("../composables/useTerminalApp", () => ({
  useTerminalApp: () => ({
    terminalAppForLaunch: { value: "Terminal" },
  }),
}));

describe("NewSessionDialog", () => {
  beforeEach(() => {
    invokeAppMock.mockReset();
  });

  it("renders with available clis and supports flex-wrap container", async () => {
    invokeAppMock.mockResolvedValue([]);

    const wrapper = mount(NewSessionDialog, {
      props: {
        initialProjectPath: "/workspace/proj1",
      },
    });

    await flushPromises();

    const cliOptions = wrapper.find(".cli-options");
    expect(cliOptions.exists()).toBe(true);
    const options = wrapper.findAll(".cli-option");
    expect(options.length).toBeGreaterThan(1);
  });

  it("loads profiles and shows selector when claude is selected and >= 2 profiles exist", async () => {
    invokeAppMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_profiles") {
        return ["official", "custom-api"];
      }
      if (cmd === "get_active_profile") {
        return "custom-api";
      }
      return null;
    });

    const wrapper = mount(NewSessionDialog, {
      props: {
        initialProjectPath: "/workspace/proj1",
        initialCliId: "claude",
      },
    });

    await flushPromises();

    const selects = wrapper.findAllComponents({ name: "ElegantSelect" });
    // First select is projectDir, second select is apiProfile
    expect(selects.length).toBe(2);
    expect(selects[1].props("options")).toEqual([
      { value: "official", label: "official", icon: "settings" },
      { value: "custom-api", label: "custom-api", icon: "settings" },
    ]);
    expect(selects[1].props("modelValue")).toBe("custom-api");

    await wrapper.find(".btn-create").trigger("click");
    expect(wrapper.emitted("create")?.[0]).toEqual([
      "/workspace/proj1",
      "claude",
      "agent",
      "custom-api",
      false,
    ]);
  });
});
