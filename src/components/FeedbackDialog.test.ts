import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import FeedbackDialog from "./FeedbackDialog.vue";
import { t } from "../i18n";

const mockDiagData = {
  appName: "SeshBuddy",
  appVersion: "0.1.0",
  tauriVersion: "2.10.1",
  os: "macos",
  osName: "macOS 26.6.2",
  osVersion: "Build 25G83",
  kernelVersion: "Darwin 25.6.0",
  arch: "aarch64",
  cpuModel: "Apple M1",
  cpuCores: 8,
  memoryTotal: "16.00 GB",
  clis: [
    { id: "claude", name: "Claude Code", command: "claude", installed: true, hasSessions: true },
    { id: "codex", name: "Codex", command: "codex", installed: true, hasSessions: false },
    { id: "workbuddy", name: "WorkBuddy", command: "workbuddy", installed: false, hasSessions: false },
  ],
};

const mocks = vi.hoisted(() => ({
  openExternalUrl: vi.fn(),
  invoke: vi.fn(),
  getVersion: vi.fn().mockResolvedValue("0.1.0"),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: mocks.getVersion,
}));

vi.mock("../utils/openExternalUrl", () => ({
  openExternalUrl: mocks.openExternalUrl,
}));

describe("FeedbackDialog 开源反馈引导弹窗", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invoke.mockResolvedValue(mockDiagData);
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("正常渲染标题、副标题与三类反馈卡片", async () => {
    const wrapper = mount(FeedbackDialog, {
      global: {
        stubs: { SvgIcon: true },
      },
    });
    await flushPromises();

    expect(wrapper.find(".feedback-title").text()).toBe(t("dialogs.feedback.title"));
    expect(wrapper.text()).toContain(t("dialogs.feedback.bugTitle"));
    expect(wrapper.text()).toContain(t("dialogs.feedback.featureTitle"));
    expect(wrapper.text()).toContain(t("dialogs.feedback.searchTitle"));
  });

  it("点击 Bug Report 卡片，携带诊断环境参数调用 openExternalUrl 并触发 close", async () => {
    const wrapper = mount(FeedbackDialog, {
      global: {
        stubs: { SvgIcon: true },
      },
    });
    await flushPromises();

    const bugCard = wrapper.findAll(".feedback-card")[0];
    await bugCard.trigger("click");

    expect(mocks.openExternalUrl).toHaveBeenCalledTimes(1);
    const openedUrl = mocks.openExternalUrl.mock.calls[0][0];
    expect(openedUrl).toContain("https://github.com/huangy7/seshbuddy/issues/new");
    expect(openedUrl).toContain("%5BBug%5D");
    expect(openedUrl).toContain("Darwin%2025.6.0");
    expect(openedUrl).toContain("macOS%2026.6.2");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("点击 Feature Request 卡片，携带功能建议模板调用 openExternalUrl 并关闭", async () => {
    const wrapper = mount(FeedbackDialog, {
      global: {
        stubs: { SvgIcon: true },
      },
    });
    await flushPromises();

    const featureCard = wrapper.findAll(".feedback-card")[1];
    await featureCard.trigger("click");

    expect(mocks.openExternalUrl).toHaveBeenCalledTimes(1);
    const openedUrl = mocks.openExternalUrl.mock.calls[0][0];
    expect(openedUrl).toContain("%5BFeature%5D");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("点击搜索已有卡片，直接跳转 Issues 列表", async () => {
    const wrapper = mount(FeedbackDialog, {
      global: {
        stubs: { SvgIcon: true },
      },
    });
    await flushPromises();

    const searchCard = wrapper.findAll(".feedback-card")[2];
    await searchCard.trigger("click");

    expect(mocks.openExternalUrl).toHaveBeenCalledTimes(1);
    expect(mocks.openExternalUrl).toHaveBeenCalledWith("https://github.com/huangy7/seshbuddy/issues");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("点击关闭按钮与遮罩背景触发 close 事件", async () => {
    const wrapper = mount(FeedbackDialog, {
      global: {
        stubs: { SvgIcon: true },
      },
    });
    await flushPromises();

    await wrapper.find(".feedback-close-btn").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);

    await wrapper.find(".modal-backdrop").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(2);
  });
});
