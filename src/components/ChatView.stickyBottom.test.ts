import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatMessage, SessionLoadResult } from "../types/session";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  ids: [] as string[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(event, handler);
    return () => mocks.listeners.delete(event);
  }),
}));

vi.mock("../composables/useSessions", async () => {
  const { ref } = await import("vue");
  return {
    useSessions: () => ({
      exportSession: vi.fn(),
      projects: ref([]),
    }),
  };
});

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
  callback(0);
  return 0;
});

vi.stubGlobal("IntersectionObserver", class {
  observe() {}
  disconnect() {}
});

const ChatView = (await import("./ChatView.vue")).default;

function message(text: string): ChatMessage {
  return {
    role: "user",
    timestamp: "2026-08-27T08:00:00Z",
    model: null,
    content_parts: [{ type: "text", text }],
  };
}

function emitChunk(requestId: string, items: ChatMessage[]) {
  const handler = mocks.listeners.get(`session:${requestId}:chunk`);
  if (!handler) throw new Error(`missing chunk listener for ${requestId}`);
  handler({ payload: items });
}

function emitDone(requestId: string, offset: number) {
  const handler = mocks.listeners.get(`session:${requestId}:done`);
  if (!handler) throw new Error(`missing done listener for ${requestId}`);
  handler({ payload: { offset, subagent_map: {} } });
}

function mountChatView(overrides: Record<string, unknown> = {}) {
  return mount(ChatView, {
    props: {
      sessionPath: "/sessions/first.jsonl",
      encodedDir: "project",
      active: true,
      showSearch: false,
      bookmarks: [],
      sessionId: "first",
      projectRoot: "/workspace/project",
      filterUser: true,
      filterAssistant: true,
      filterTool: true,
      filterThinking: true,
      cliId: "claude",
      ...overrides,
    },
    global: {
      stubs: {
        LoadingSkeleton: true,
        ChatSearchBar: true,
        ChatApiDetailPanel: true,
        ChatImageFullscreenModal: true,
        ChatMessageBody: true,
        ChatSelectionPill: true,
        SvgIcon: true,
      },
    },
  });
}

describe("ChatView Adaptive Sticky Bottom & Live Append", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout"] });
    mocks.listeners.clear();
    mocks.invoke.mockReset();
    mocks.ids = ["first-request", "second-request"];
    vi.stubGlobal("crypto", {
      randomUUID: vi.fn(() => mocks.ids.shift() ?? "fallback-request"),
    });
    HTMLElement.prototype.scrollTo = vi.fn();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("automatically starts live watch and increments unreadCount when away from bottom", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "load_session_stream") {
        return undefined;
      }
      if (command === "proxy_status") {
        return { enabled: false, running: false };
      }
      if (command === "load_session_incremental") {
        return {
          messages: [message("new live message 1"), message("new live message 2")],
          offset: 150,
          subagent_map: {},
        } as SessionLoadResult;
      }
      return undefined;
    });

    const wrapper = mountChatView();
    await flushPromises();

    // Initial load: 2 messages
    emitChunk("first-request", [message("msg 1"), message("msg 2")]);
    emitDone("first-request", 100);
    await nextTick();

    // Verify initial state: 2 messages
    expect(wrapper.vm.messages.length).toBe(2);

    // Mock scrolling away from bottom (scrollTop: 100, scrollHeight: 1000, clientHeight: 400 -> distanceFromBottom: 500)
    const el = wrapper.find(".chat-view").element as HTMLElement;
    Object.defineProperty(el, "scrollHeight", { value: 1000, configurable: true });
    Object.defineProperty(el, "clientHeight", { value: 400, configurable: true });
    Object.defineProperty(el, "scrollTop", { value: 100, configurable: true });
    el.dispatchEvent(new Event("scroll"));
    await nextTick();

    // Simulate 2s liveWatch interval firing
    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();

    // Verify incremental load was called
    expect(mocks.invoke).toHaveBeenCalledWith("load_session_incremental", {
      filePath: "/sessions/first.jsonl",
      offset: 100,
      skipSidechainFilter: false,
    });

    // 2 new messages appended -> total 4
    expect(wrapper.vm.messages.length).toBe(4);

    // Since user was browsing history away from bottom, unread badge pill appears
    const scrollBtn = wrapper.findComponent({ name: "ScrollToBottom" });
    expect(scrollBtn.exists()).toBe(true);
    expect(scrollBtn.props("unreadCount")).toBe(2);

    // Clicking scroll to bottom clears unread count
    await scrollBtn.find(".scroll-btn").trigger("click");
    await nextTick();
    expect(scrollBtn.props("unreadCount")).toBe(0);
  });
});
