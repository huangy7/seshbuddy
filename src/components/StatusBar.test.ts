import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import StatusBar from "./StatusBar.vue";
import { t } from "../i18n";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

function mountStatusBar(props: Record<string, unknown> = {}) {
  return mount(StatusBar, {
    props: {
      projectCount: 51,
      sessionCount: 660,
      messageCount: null,
      cliId: "claude",
      ...props,
    },
    global: {
      stubs: {
        SvgIcon: {
          template: '<span class="svg-icon-stub" :data-name="name" :class="$attrs.class" />',
          props: ["name", "size"],
        },
      },
    },
  });
}

describe("StatusBar 会话刷新胶囊按钮交互", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue({ enabled: false, running: false });
    mocks.listen.mockReset();
    mocks.listen.mockResolvedValue(() => {});
  });

  it("当 messageCount 为 null 时，不渲染刷新胶囊按钮", () => {
    const wrapper = mountStatusBar({ messageCount: null });
    expect(wrapper.find(".status-session-refresh-btn").exists()).toBe(false);
  });

  it("当 messageCount 有数值时，渲染胶囊按钮与消息数文案", () => {
    const wrapper = mountStatusBar({ messageCount: 11 });
    const btn = wrapper.find(".status-session-refresh-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.text()).toContain(t("session.common.messageCount", { count: 11 }));
    expect(btn.attributes("title")).toContain(t("session.statusBar.reloadTitle"));
    expect(btn.attributes("disabled")).toBeUndefined();
  });

  it("点击胶囊按钮时，触发 refreshSession 事件", async () => {
    const wrapper = mountStatusBar({ messageCount: 11, refreshing: false });
    const btn = wrapper.find(".status-session-refresh-btn");
    await btn.trigger("click");
    expect(wrapper.emitted("refreshSession")).toHaveLength(1);
  });

  it("当 refreshing 为 true 时，按钮置为 disabled 且图标具有 spin class", () => {
    const wrapper = mountStatusBar({ messageCount: 11, refreshing: true });
    const btn = wrapper.find(".status-session-refresh-btn");
    expect(btn.attributes("disabled")).toBeDefined();
    expect(btn.attributes("title")).toContain(t("session.common.reloading"));
    const icon = btn.find(".refresh-icon");
    expect(icon.classes()).toContain("spin");
  });
});
