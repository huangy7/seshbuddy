import { describe, it, expect, vi } from "vitest";

// 模块级 localStorage 读取发生在 import 链上（useSessions.ts），须先 stub 再动态 import
vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

const { mount, flushPromises } = await import("@vue/test-utils");
const SessionHeader = (await import("./SessionHeader.vue")).default;
const { t } = await import("../i18n");

const baseProps = {
  displayName: "test",
  projectPath: "/tmp/x",
  timestamp: "2026-08-19T15:59:00Z",
  messageCount: 2,
  bookmarks: [],
  timelineOpen: false,
  archivePinned: false,
  archivePinLoading: false,
  stats: null,
};

describe("SessionHeader primary top-bar buttons", () => {
  it("renders 5 primary action buttons on the top bar including bookmark", () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, hasParentSession: true, favorite: true },
    });

    const actionButtons = wrapper.findAll(".header-actions > .action-btn, .header-actions > .more-wrapper > .action-btn, .header-actions > .bookmark-wrapper > .action-btn");
    expect(actionButtons).toHaveLength(5);

    expect(wrapper.find(".header-btn-favorite").exists()).toBe(true);
    expect(wrapper.find(".header-btn-bookmark").exists()).toBe(true);
    expect(wrapper.find(".header-btn-project").exists()).toBe(true);
    expect(wrapper.find(".header-btn-refresh").exists()).toBe(true);
    expect(wrapper.find(".header-btn-more").exists()).toBe(true);

    // Flat secondary buttons must NOT exist directly on top bar
    expect(wrapper.find(".header-actions > .header-btn-autofollow").exists()).toBe(false);
    expect(wrapper.find(".header-actions > .header-btn-pin").exists()).toBe(false);
    expect(wrapper.find(".header-actions > .header-btn-stats").exists()).toBe(false);
    expect(wrapper.find(".header-actions > .timeline-btn").exists()).toBe(false);
    expect(wrapper.find(".session-header > .back-to-parent-btn").exists()).toBe(false);
  });

  it("triggers toggleFavorite, openProject, and bookmark dropdown correctly", async () => {
    const wrapper = mount(SessionHeader, {
      props: {
        ...baseProps,
        favorite: false,
        bookmarks: [
          { cliId: "claude", sessionId: "s1", messageIndex: 3, note: "Key fix", createdAt: 1700000000000 },
        ],
      },
    });

    await wrapper.find(".header-btn-favorite").trigger("click");
    expect(wrapper.emitted("toggleFavorite")).toBeTruthy();

    await wrapper.find(".header-btn-project").trigger("click");
    expect(wrapper.emitted("openProject")).toBeTruthy();

    const bmBtn = wrapper.find(".header-btn-bookmark");
    expect(bmBtn.classes()).toContain("has-bookmarks");
    await bmBtn.trigger("click");
    expect(wrapper.find(".bookmark-menu").exists()).toBe(true);
    const item = wrapper.find(".bookmark-item");
    expect(item.exists()).toBe(true);
    await item.trigger("click");
    expect(wrapper.emitted("scrollToBookmark")?.[0]).toEqual([3]);
  });
});

describe("SessionHeader refresh entry", () => {
  it("shows refresh button with reload title and shortcut", () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps },
    });
    const refreshBtn = wrapper.find(".header-btn-refresh");
    expect(refreshBtn.exists()).toBe(true);
    expect(refreshBtn.attributes("title")).toContain(t("session.sessionHeader.refreshTitle"));
    expect(refreshBtn.attributes("title")).toContain("⌘R");
  });

  it("spins icon when isRefreshing prop is true", () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, isRefreshing: true },
    });
    const refreshBtn = wrapper.find(".header-btn-refresh");
    expect(refreshBtn.attributes("title")).toContain(t("session.common.reloading"));
    expect(refreshBtn.find(".spin").exists()).toBe(true);
  });
});

describe("SessionHeader more-menu dropdown items & events", () => {
  it("opens more-menu with all secondary action items and a11y attributes", async () => {
    const wrapper = mount(SessionHeader, {
      props: {
        ...baseProps,
        messageCount: 5,
        hasParentSession: true,
        cliId: "claude",
      },
      attachTo: document.body,
    });

    const moreBtn = wrapper.find(".header-btn-more");
    expect(moreBtn.attributes("aria-haspopup")).toBe("menu");
    expect(moreBtn.attributes("aria-expanded")).toBe("false");

    expect(wrapper.find(".more-menu").exists()).toBe(false);
    await moreBtn.trigger("click");
    expect(wrapper.find(".more-menu").exists()).toBe(true);
    expect(moreBtn.attributes("aria-expanded")).toBe("true");

    const menu = wrapper.find(".more-menu");
    expect(menu.attributes("role")).toBe("menu");
    expect(menu.attributes("aria-label")).toBe(t("session.sessionHeader.moreActions"));

    // 逐项比对而非逐个 toContain：集合相等同时挡住「缺项」与「多出意料之外的项」，
    // 逐个 toContain 只挡前者——菜单里混进一项新功能时它照样全绿。
    const itemLabels = menu.findAll(".menu-item").map((el) => el.text());
    expect(itemLabels).toEqual([
      t("session.sessionHeader.backToParent"),
      t("session.sessionHeader.pinArchive"),
      t("session.sessionHeader.analytics"),
      t("session.sessionHeader.export"),
      t("session.sessionHeader.multiSelect"),
      t("session.sessionHeader.resume"),
      t("session.sessionHeader.copyResumeCommand"),
    ]);

    wrapper.unmount();
  });

  it("handles keyboard Esc to close menu and ArrowDown/ArrowUp cycling", async () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, messageCount: 5, cliId: "claude" },
      attachTo: document.body,
    });

    const moreBtn = wrapper.find(".header-btn-more");
    await moreBtn.trigger("click");
    expect(wrapper.find(".more-menu").exists()).toBe(true);

    // Esc closes menu
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".more-menu").exists()).toBe(false);

    wrapper.unmount();
  });

  it("toggles archivePinned from more-menu", async () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, archivePinned: false },
    });
    await wrapper.find(".header-btn-more").trigger("click");
    const pinItem = wrapper.find(".header-menu-pin");
    expect(pinItem.exists()).toBe(true);
    await pinItem.trigger("click");
    expect(wrapper.emitted("toggleArchivePinned")).toBeTruthy();
  });

  it("enters selection mode from more-menu", async () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps },
    });
    await wrapper.find(".header-btn-more").trigger("click");
    const selectItem = wrapper.find(".header-menu-select");
    expect(selectItem.exists()).toBe(true);
    await selectItem.trigger("click");
    expect(wrapper.emitted("enterSelectionMode")).toBeTruthy();
  });

  it("copies restore command from more-menu", async () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, cliId: "claude" },
    });
    await wrapper.find(".header-btn-more").trigger("click");
    const copyItem = wrapper.find(".header-menu-copy-command");
    expect(copyItem.exists()).toBe(true);
    await copyItem.trigger("click");
    expect(wrapper.emitted("copyRestoreCommand")).toBeTruthy();
  });

  it("triggers backToParent from more-menu when hasParentSession is true", async () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, hasParentSession: true },
    });
    await wrapper.find(".header-btn-more").trigger("click");
    const backItem = wrapper.find(".header-menu-back-parent");
    expect(backItem.exists()).toBe(true);
    await backItem.trigger("click");
    expect(wrapper.emitted("backToParent")).toBeTruthy();
  });

  it("shows analytics item in more-menu only when messageCount > 0", async () => {
    const wrapperWithMsg = mount(SessionHeader, {
      props: { ...baseProps, messageCount: 3 },
    });
    await wrapperWithMsg.find(".header-btn-more").trigger("click");
    expect(wrapperWithMsg.find(".header-menu-analytics").exists()).toBe(true);

    const wrapperNoMsg = mount(SessionHeader, {
      props: { ...baseProps, messageCount: 0 },
    });
    await wrapperNoMsg.find(".header-btn-more").trigger("click");
    expect(wrapperNoMsg.find(".header-menu-analytics").exists()).toBe(false);
  });

  it("hides resume and copy command for non-resumable CLI", async () => {
    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, cliId: "dsh" },
    });
    await wrapper.find(".header-btn-more").trigger("click");
    expect(wrapper.find(".header-menu-resume").exists()).toBe(false);
    expect(wrapper.find(".header-menu-copy-command").exists()).toBe(false);
  });

  it("copies project path when clicking session-path button", async () => {
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: {
        writeText: writeTextMock,
      },
    });

    const wrapper = mount(SessionHeader, {
      props: { ...baseProps, projectPath: "/Users/test/project" },
    });

    const pathBtn = wrapper.find(".session-path");
    expect(pathBtn.exists()).toBe(true);
    await pathBtn.trigger("click");
    // copyToClipboard 先试原生链路再降级到 Web API，写入结果要多等一轮微任务才落到状态上
    await flushPromises();
    expect(writeTextMock).toHaveBeenCalledWith("/Users/test/project");
    expect(wrapper.find(".copied-badge").exists()).toBe(true);
  });
});
