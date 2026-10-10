import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import ResumeSessionDialog from "./ResumeSessionDialog.vue";

const invokeAppMock = vi.fn();
vi.mock("../utils/invokeApp", () => ({
  invokeApp: (cmd: string, args: any) => invokeAppMock(cmd, args),
}));

vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({
    skipPermissions: { value: false },
  }),
}));

describe("ResumeSessionDialog", () => {
  beforeEach(() => {
    invokeAppMock.mockReset();
  });

  it("loads Claude profiles on mount and displays selector when >= 2 profiles exist", async () => {
    invokeAppMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_profiles") {
        return ["default", "deepseek", "kimi"];
      }
      if (cmd === "get_active_profile") {
        return "deepseek";
      }
      return null;
    });

    const wrapper = mount(ResumeSessionDialog, {
      props: {
        cliId: "claude",
      },
    });

    await flushPromises();

    expect(wrapper.find(".form-group:nth-of-type(2)").exists()).toBe(true);
    const select = wrapper.findComponent({ name: "ElegantSelect" });
    expect(select.exists()).toBe(true);
    expect(select.props("options")).toEqual([
      { value: "default", label: "default", icon: "settings" },
      { value: "deepseek", label: "deepseek", icon: "settings" },
      { value: "kimi", label: "kimi", icon: "settings" },
    ]);
    expect(select.props("modelValue")).toBe("deepseek");

    await wrapper.find(".btn-primary").trigger("click");
    expect(wrapper.emitted("resume")?.[0]).toEqual(["agent", "deepseek", false]);
  });

  it("does not display profile selector for non-claude cli", async () => {
    const wrapper = mount(ResumeSessionDialog, {
      props: {
        cliId: "goose",
        profiles: ["default", "custom"],
      },
    });

    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).exists()).toBe(false);

    await wrapper.find(".btn-primary").trigger("click");
    expect(wrapper.emitted("resume")?.[0]).toEqual(["agent", null, false]);
  });

  it("updates profiles when props.profiles changes dynamically", async () => {
    invokeAppMock.mockResolvedValue([]);

    const wrapper = mount(ResumeSessionDialog, {
      props: {
        cliId: "claude",
        profiles: [],
      },
    });

    await flushPromises();
    expect(wrapper.findComponent({ name: "ElegantSelect" }).exists()).toBe(false);

    await wrapper.setProps({
      profiles: ["work", "personal"],
    });
    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).exists()).toBe(true);
    const select = wrapper.findComponent({ name: "ElegantSelect" });
    expect(select.props("options")).toHaveLength(2);
  });
});
