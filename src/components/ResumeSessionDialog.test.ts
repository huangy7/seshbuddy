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

function mockProfiles(names: string[], active: string | (() => never)) {
  invokeAppMock.mockImplementation(async (cmd: string) => {
    if (cmd === "list_profiles") return names;
    if (cmd === "get_active_profile") {
      if (typeof active === "function") return active();
      return active;
    }
    return null;
  });
}

describe("ResumeSessionDialog", () => {
  beforeEach(() => {
    invokeAppMock.mockReset();
  });

  it("loads Claude profiles on mount and displays selector when >= 2 profiles exist", async () => {
    mockProfiles(["default", "deepseek", "kimi"], "deepseek");

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
      },
    });

    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).exists()).toBe(false);

    await wrapper.find(".btn-primary").trigger("click");
    expect(wrapper.emitted("resume")?.[0]).toEqual(["agent", null, false]);
  });

  it("falls back to the first fetched profile when the active profile lookup fails", async () => {
    mockProfiles(["alpha", "beta"], () => {
      throw new Error("boom");
    });

    const wrapper = mount(ResumeSessionDialog, {
      props: { cliId: "claude" },
    });

    await flushPromises();

    const select = wrapper.findComponent({ name: "ElegantSelect" });
    expect(select.props("options")).toHaveLength(2);
    expect(select.props("modelValue")).toBe("alpha");
  });

  it("falls back to the first fetched profile when the active profile is not in the list", async () => {
    mockProfiles(["alpha", "beta"], "deleted-profile");

    const wrapper = mount(ResumeSessionDialog, {
      props: { cliId: "claude" },
    });

    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).props("modelValue")).toBe("alpha");
  });

  it("shows no selector and emits a null profile when listing profiles fails", async () => {
    invokeAppMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_profiles") throw new Error("db busy");
      return null;
    });

    const wrapper = mount(ResumeSessionDialog, {
      props: { cliId: "claude" },
    });

    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).exists()).toBe(false);

    await wrapper.find(".btn-primary").trigger("click");
    expect(wrapper.emitted("resume")?.[0]).toEqual(["agent", null, false]);
  });

  it("does not show a stale parent-supplied list when listing profiles fails", async () => {
    invokeAppMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_profiles") throw new Error("db busy");
      return null;
    });

    const wrapper = mount(ResumeSessionDialog, {
      props: { cliId: "claude", profiles: ["stale-a", "stale-b"] },
    });

    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).exists()).toBe(false);
  });

  it("does not preselect a stale parent-supplied profile when the active lookup fails", async () => {
    mockProfiles(["fresh-a", "fresh-b"], () => {
      throw new Error("boom");
    });

    const wrapper = mount(ResumeSessionDialog, {
      props: { cliId: "claude", profiles: ["stale-a", "stale-b"] },
    });

    await flushPromises();

    expect(wrapper.findComponent({ name: "ElegantSelect" }).props("modelValue")).toBe("fresh-a");
  });

  it("keeps the fetched list when the parent re-supplies an older list afterwards", async () => {
    mockProfiles(["fresh-a", "fresh-b"], "fresh-a");

    const wrapper = mount(ResumeSessionDialog, {
      props: { cliId: "claude", profiles: ["stale-a", "stale-b"] },
    });
    await flushPromises();

    await wrapper.setProps({ profiles: ["stale-a", "stale-b"] });
    await flushPromises();

    const values = (wrapper.findComponent({ name: "ElegantSelect" }).props("options") as any[]).map(
      (o) => o.value
    );
    expect(values).toEqual(["fresh-a", "fresh-b"]);
  });

  it("refetches the list on every mount instead of reusing an earlier result", async () => {
    mockProfiles(["alpha", "beta"], "alpha");
    const first = mount(ResumeSessionDialog, { props: { cliId: "claude" } });
    await flushPromises();
    expect(first.findComponent({ name: "ElegantSelect" }).props("options")).toHaveLength(2);
    first.unmount();

    mockProfiles(["alpha", "beta", "gamma"], "gamma");
    const second = mount(ResumeSessionDialog, { props: { cliId: "claude" } });
    await flushPromises();
    const select = second.findComponent({ name: "ElegantSelect" });
    expect(select.props("options")).toHaveLength(3);
    expect(select.props("modelValue")).toBe("gamma");
  });
});
