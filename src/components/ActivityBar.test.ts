import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { t } from "../i18n";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

let store: Record<string, string> = {};

vi.stubGlobal("localStorage", {
  getItem: (key: string) => store[key] ?? null,
  setItem: (key: string, val: string) => { store[key] = val; },
  removeItem: (key: string) => { delete store[key]; },
  clear: () => { store = {}; },
});

const { mount } = await import("@vue/test-utils");
const ActivityBar = (await import("./ActivityBar.vue")).default;

function mountActivityBar(props: Record<string, unknown> = {}) {
  return mount(ActivityBar, {
    attachTo: document.body,
    props: {
      activeMode: "history",
      sidebarCollapsed: false,
      showUsage: true,
      showApiDebug: true,
      ...props,
    },
    global: {
      stubs: { SvgIcon: true },
    },
  });
}

describe("ActivityBar rail 结构与经典垂直箭头折叠展开", () => {
  beforeEach(() => {
    store = {};
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(null);
    mocks.listen.mockReset();
    mocks.listen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("默认收起态 (48px) 下只渲染图标，不渲染文字标签", () => {
    const wrapper = mountActivityBar();
    expect(wrapper.find(".item-label").exists()).toBe(false);
    expect(wrapper.find(".arrow-toggle-btn").exists()).toBe(true);
  });

  it("展开态下渲染两字精简标签", async () => {
    store["seshbuddy-activity-bar-expanded"] = "true";
    const wrapper = mountActivityBar();
    
    expect(wrapper.classes()).toContain("expanded");
    const labels = wrapper.findAll(".item-label").map((el) => el.text());
    expect(labels).toContain(t("session.activityBar.tabHistory"));
    expect(labels).toContain(t("session.favoritesPanel.title"));
    expect(labels).toContain(t("session.bookmarkList.title"));
    expect(labels).toContain(t("session.activityBar.tabAgent"));
    expect(labels).toContain(t("session.activityBar.usageShort"));
    expect(labels).toContain(t("app.filter.assistant"));
    expect(labels).toContain(t("session.activityBar.settings"));
  });

  it("用量统计常驻在底部组且不折叠，点击箭头按钮就地展开/收起次要工具（调试、反馈）", async () => {
    const wrapper = mountActivityBar();
    const arrowBtn = wrapper.find(".arrow-toggle-btn");
    expect(arrowBtn.exists()).toBe(true);

    // 用量统计常驻且未折叠
    expect(wrapper.find(`[title="${t("session.usageDashboard.title")}"]`).exists()).toBe(true);
    await wrapper.find(`[title="${t("session.usageDashboard.title")}"]`).trigger("click");
    expect(wrapper.emitted("openUsage")).toHaveLength(1);

    // 默认未展开底部次要工具
    expect(wrapper.find(`[title="${t("dialogs.apiDebug.title")}"]`).exists()).toBe(false);

    // 点击箭头展开
    await arrowBtn.trigger("click");
    expect(wrapper.find(`[title="${t("dialogs.apiDebug.title")}"]`).exists()).toBe(true);
    expect(wrapper.find(`[title="${t("app.about.feedbackTitle")}"]`).exists()).toBe(true);

    // 点击反馈触发 emit
    await wrapper.find(`[title="${t("app.about.feedbackTitle")}"]`).trigger("click");
    expect(wrapper.emitted("openFeedback")).toHaveLength(1);

    // 再次点击箭头收起
    await arrowBtn.trigger("click");
    expect(wrapper.find(`[title="${t("dialogs.apiDebug.title")}"]`).exists()).toBe(false);
  });

  it("点击核心工具与设置按钮正常触发对应 emit", async () => {
    const wrapper = mountActivityBar();

    await wrapper.find(`[title="${t("native.assistantWindowTitle")}"]`).trigger("click");
    expect(wrapper.emitted("openAssistant")).toHaveLength(1);


    await wrapper.find(`[title="${t("session.activityBar.settings")}"]`).trigger("click");
    expect(wrapper.emitted("openSettings")).toHaveLength(1);
  });
});
