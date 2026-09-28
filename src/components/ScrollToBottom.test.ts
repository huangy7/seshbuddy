import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ScrollToBottom from "./ScrollToBottom.vue";
import { t } from "../i18n";

function createMockContainer(opts: { scrollTop?: number; scrollHeight?: number; clientHeight?: number } = {}) {
  const div = document.createElement("div");
  Object.defineProperty(div, "scrollTop", { value: opts.scrollTop ?? 0, writable: true });
  Object.defineProperty(div, "scrollHeight", { value: opts.scrollHeight ?? 1000, writable: true });
  Object.defineProperty(div, "clientHeight", { value: opts.clientHeight ?? 400, writable: true });
  return div;
}

describe("ScrollToBottom", () => {
  it("is hidden by default when container is short with no unread messages", () => {
    const container = createMockContainer({ scrollTop: 0, scrollHeight: 450, clientHeight: 400 });
    const wrapper = mount(ScrollToBottom, {
      props: { container, unreadCount: 0 },
      global: {
        stubs: { SvgIcon: true },
      },
    });

    expect(wrapper.find(".scroll-btn").exists()).toBe(false);
  });

  it("becomes visible as a pill badge when unreadCount > 0", () => {
    const container = createMockContainer({ scrollTop: 0, scrollHeight: 1000, clientHeight: 400 });
    const wrapper = mount(ScrollToBottom, {
      props: { container, unreadCount: 5 },
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const btn = wrapper.find(".scroll-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.classes()).toContain("has-unread");
    expect(btn.text()).toContain(t("common.scrollToBottom.newMessages", { count: 5 }));
  });

  it("caps unread display at 99+", () => {
    const container = createMockContainer({ scrollTop: 0, scrollHeight: 1000, clientHeight: 400 });
    const wrapper = mount(ScrollToBottom, {
      props: { container, unreadCount: 150 },
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const btn = wrapper.find(".scroll-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.text()).toContain(t("common.scrollToBottom.newMessagesCapped"));
  });

  it("emits scrollToEnd when clicked with unreadCount > 0", async () => {
    const container = createMockContainer({ scrollTop: 100, scrollHeight: 1000, clientHeight: 400 });
    const wrapper = mount(ScrollToBottom, {
      props: { container, unreadCount: 3 },
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const btn = wrapper.find(".scroll-btn");
    await btn.trigger("click");
    expect(wrapper.emitted("scrollToEnd")).toBeTruthy();
  });

  it("is visible when scrolled down without unread messages", () => {
    const container = createMockContainer({ scrollTop: 300, scrollHeight: 1000, clientHeight: 400 });
    const wrapper = mount(ScrollToBottom, {
      props: { container, unreadCount: 0 },
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const btn = wrapper.find(".scroll-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.classes()).not.toContain("has-unread");
  });
});
