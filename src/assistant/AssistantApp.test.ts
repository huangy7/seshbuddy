import { describe, it, expect, vi, beforeEach } from "vitest";

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

const mocks = vi.hoisted(() => ({
  send: vi.fn(),
  setDraft: vi.fn(),
  // assistant_take_pending_prompt 的依次返回值（take 语义：消费后即空）
  pendingPrompts: [] as (string | { text: string; autoSend?: boolean } | null)[],
  listenHandlers: new Map<string, (e: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "assistant_take_pending_prompt") {
      return mocks.pendingPrompts.shift() ?? null;
    }
    if (cmd === "assistant_engine_status") {
      return { available: true };
    }
    return null;
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    mocks.listenHandlers.set(event, handler);
    return () => {
      mocks.listenHandlers.delete(event);
    };
  }),
}));

vi.mock("../composables/useAssistantChat", () => ({
  useAssistantChat: () => ({
    messages: { value: [] },
    running: { value: false },
    activity: { value: null },
    conversationId: { value: null },
    conversationUsage: { value: null },
    send: mocks.send,
    cancel: vi.fn(),
    loadConversation: vi.fn(),
    newConversation: vi.fn(),
    dispose: vi.fn(),
  }),
}));

const { mount, flushPromises } = await import("@vue/test-utils");
const AssistantApp = (await import("./AssistantApp.vue")).default;

// 模拟 ProfilePicker mount 后异步恢复上次选择
const ProfilePickerStub = {
  props: ["modelValue"],
  emits: ["update:modelValue"],
  mounted(this: { $emit: (e: string, v: string) => void }) {
    this.$emit("update:modelValue", "test-profile");
  },
  template: "<div />",
};

const ChatInputStub = {
  template: "<div />",
  methods: {
    setDraft(text: string) {
      mocks.setDraft(text);
    },
  },
};

function mountApp() {
  return mount(AssistantApp, {
    global: {
      stubs: {
        ProfilePicker: ProfilePickerStub,
        ChatMessageList: true,
        ChatInput: ChatInputStub,
        ConversationHistory: true,
        CacheBackfillBar: true,
      },
    },
  });
}

describe("AssistantApp 消费 Dashboard 发来的 prompt", () => {
  beforeEach(() => {
    mocks.send.mockClear();
    mocks.setDraft.mockClear();
    mocks.pendingPrompts = [];
    mocks.listenHandlers.clear();
  });

  it("mount 时取走 pending prompt 并交给 send（使用当前 profile/model）", async () => {
    mocks.pendingPrompts = ["帮我总结本周工作"];
    mountApp();
    await flushPromises();
    await flushPromises();
    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(mocks.send).toHaveBeenCalledWith("帮我总结本周工作", "test-profile", undefined);
  });

  it("autoSend 为 false 时预填到输入框而不调用 send", async () => {
    mocks.pendingPrompts = [{ text: "预填的周报 prompt", autoSend: false }];
    mountApp();
    await flushPromises();
    await flushPromises();
    expect(mocks.send).not.toHaveBeenCalled();
    expect(mocks.setDraft).toHaveBeenCalledWith("预填的周报 prompt");
  });

  it("窗口已开时经 assistant-dashboard-prompt 事件取走 prompt，且只消费一次", async () => {
    mountApp();
    await flushPromises();
    expect(mocks.send).not.toHaveBeenCalled();

    const handler = mocks.listenHandlers.get("assistant-dashboard-prompt");
    expect(handler).toBeDefined();

    // 事件触发后 take 到 prompt → send 一次
    mocks.pendingPrompts = ["查一下最近对话"];
    handler!({ payload: null });
    await flushPromises();
    await flushPromises();
    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(mocks.send).toHaveBeenCalledWith("查一下最近对话", "test-profile", undefined);

    // 再次触发（或重复 mount 取数）已无 pending prompt → 不重复发送
    handler!({ payload: null });
    await flushPromises();
    expect(mocks.send).toHaveBeenCalledTimes(1);
  });
});
