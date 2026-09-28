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
      autoFollow: false,
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
        ScrollToBottom: true,
        ChatSearchBar: true,
        ChatApiDetailPanel: true,
        ChatImageFullscreenModal: true,
        ChatMessageBody: true,
        ChatMessageHeader: true,
        ChatSelectionPill: true,
        ChatToolGroup: true,
        ChatAvatar: true,
        ChatTimeDivider: true,
        SvgIcon: true,
      },
    },
  });
}

function streamCalls() {
  return mocks.invoke.mock.calls.filter(([command]) => command === "load_session_stream");
}

describe("ChatView preview session identity", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "proxy_status") return { enabled: false, running: false };
      return undefined;
    });
    mocks.listeners.clear();
    mocks.ids = ["first-request", "second-request", "third-request"];
    vi.stubGlobal("crypto", {
      randomUUID: vi.fn(() => mocks.ids.shift() ?? "fallback-request"),
    });
    HTMLElement.prototype.scrollTo = vi.fn();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("reloads an already-mounted preview when only its path changes", async () => {
    const wrapper = mountChatView();
    await flushPromises();

    expect(streamCalls()).toEqual([
      ["load_session_stream", {
        requestId: "first-request",
        filePath: "/sessions/first.jsonl",
        cliId: "claude",
        skipSidechainFilter: false,
      }],
    ]);

    emitChunk("first-request", [message("first conversation")]);
    await nextTick();
    expect(wrapper.vm.messages.map((item: ChatMessage) => item.content_parts[0])).toEqual([
      { type: "text", text: "first conversation" },
    ]);

    const staleChunk = mocks.listeners.get("session:first-request:chunk");
    expect(staleChunk).toBeTypeOf("function");

    await wrapper.setProps({
      sessionPath: "/sessions/second.jsonl",
      sessionId: "second",
    });
    await flushPromises();

    expect(streamCalls()).toHaveLength(2);
    expect(streamCalls()[1]).toEqual([
      "load_session_stream",
      {
        requestId: "second-request",
        filePath: "/sessions/second.jsonl",
        cliId: "claude",
        skipSidechainFilter: false,
      },
    ]);
    expect(wrapper.vm.messages).toEqual([]);

    staleChunk?.({ payload: [message("stale first conversation")] });
    emitChunk("second-request", [message("second conversation")]);
    await nextTick();

    expect(wrapper.vm.messages.map((item: ChatMessage) => item.content_parts[0])).toEqual([
      { type: "text", text: "second conversation" },
    ]);
    wrapper.unmount();
  });

  it("discards a delayed incremental response from the previous identity", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout"] });
    let resolveOldIncremental!: (result: SessionLoadResult) => void;
    const oldIncremental = new Promise<SessionLoadResult>((resolve) => {
      resolveOldIncremental = resolve;
    });
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "proxy_status") return { enabled: false, running: false };
      if (command === "load_session_incremental" && args?.filePath === "/sessions/first.jsonl") {
        return oldIncremental;
      }
      if (command === "load_session_incremental") {
        return { messages: [], offset: 21, subagent_map: {} };
      }
      return undefined;
    });

    const wrapper = mountChatView({ autoFollow: true });
    await flushPromises();
    emitDone("first-request", 10);
    await nextTick();
    await vi.advanceTimersByTimeAsync(2000);

    expect(mocks.invoke).toHaveBeenCalledWith("load_session_incremental", {
      filePath: "/sessions/first.jsonl",
      offset: 10,
      skipSidechainFilter: false,
    });

    await wrapper.setProps({
      sessionPath: "/sessions/second.jsonl",
      sessionId: "second",
    });
    await flushPromises();
    emitDone("second-request", 20);
    await nextTick();

    resolveOldIncremental({
      messages: [message("stale incremental conversation")],
      offset: 999,
      subagent_map: { stale: { file_path: "/stale.jsonl", label: "stale" } },
    });
    await flushPromises();
    expect(wrapper.vm.messages).toEqual([]);

    await vi.advanceTimersByTimeAsync(2000);
    const incrementalCalls = mocks.invoke.mock.calls.filter(
      ([command]) => command === "load_session_incremental",
    );
    expect(incrementalCalls.at(-1)?.[1]).toMatchObject({
      filePath: "/sessions/second.jsonl",
      offset: 20,
    });
    wrapper.unmount();
  });

  it("preserves scroll position when the same pinned identity is hidden and shown", async () => {
    const wrapper = mountChatView();
    await flushPromises();

    const chatContainer = wrapper.get<HTMLElement>(".chat-view").element;
    chatContainer.scrollTop = 340;

    await wrapper.setProps({ active: false });
    await wrapper.setProps({ active: true });
    await flushPromises();

    expect(streamCalls()).toHaveLength(1);

    expect(chatContainer.scrollTop).toBe(340);
    wrapper.unmount();
  });

  it("reloads when only the CLI changes", async () => {
    const wrapper = mountChatView();
    await flushPromises();

    await wrapper.setProps({ cliId: "codex" });
    await flushPromises();

    expect(streamCalls()).toHaveLength(2);
    expect(streamCalls()[1][1]).toMatchObject({
      filePath: "/sessions/first.jsonl",
      cliId: "codex",
    });
    wrapper.unmount();
  });
});
