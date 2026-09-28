import { describe, it, expect, vi, beforeEach } from "vitest";
import { nextTick } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ChatMessageList from "./ChatMessageList.vue";
import type { ChatMessage } from "../../composables/useAssistantChat";
import { t, setLocale } from "../../i18n";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue([]),
}));

// 剪贴板写入的**原生那一层**要能被逐用例控制成败：不 mock 它的话，`invoke` 的默认桩
// 会让原生写入恒成功，于是「复制失败」这条路径根本走不到。
const clipboardWrite = vi.hoisted(() => ({ writeText: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: clipboardWrite.writeText,
}));

describe("ChatMessageList", () => {
  // 逐用例复位后端桩：有用例会把 invoke 改成返回 `title: null` 的引用解析结果，
  // 不复位的话它后面的用例会继承那个桩（当前它是最后一个，靠顺序侥幸不泄漏）。
  beforeEach(() => {
    vi.mocked(invoke).mockResolvedValue([]);
  });

  it("渲染空状态并能点击快捷建议", async () => {
    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [],
        activity: null,
        profile: "test-profile",
      },
    });

    expect(wrapper.find(".chat-empty").exists()).toBe(true);
    expect(wrapper.text()).toContain(t("assistant.messageList.emptyTitle"));
    expect(wrapper.text()).toContain(
      t("assistant.messageList.emptyPinHint", { profile: "test-profile" }),
    );

    const chips = wrapper.findAll(".suggestion-chip");
    expect(chips.length).toBeGreaterThan(0);
    await chips[0].trigger("click");
    expect(wrapper.emitted("pick")).toBeTruthy();
  });

  it("渲染 02 Thinking & 05 Tool Chips 原语化卡片与展开切换", async () => {
    const processMessage: ChatMessage = {
      role: "assistant",
      kind: "process",
      text: "",
      collapsed: true,
      durationMs: 3400,
      steps: [
        {
          kind: "tool",
          actionId: "search",
          targets: ["workspace/SeshBuddy", "sessions"],
          count: 2,
          done: true,
        },
        {
          kind: "note",
          text: "正在提炼周报核心要点",
          done: true,
        },
      ],
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [processMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    const pill = wrapper.find(".thinking-pill");
    expect(pill.exists()).toBe(true);
    expect(pill.text()).toContain(t("assistant.messageList.thoughtFor", { duration: "3.4s" }));

    // 默认折叠时未展开工具芯片
    expect(wrapper.find(".tool-chips-container").exists()).toBe(false);

    // 点击展开
    await pill.trigger("click");
    expect(wrapper.find(".tool-chips-container").exists()).toBe(true);

    const rows = wrapper.findAll(".tool-chip-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("Search conversation content");
    // 搜索步骤的目标带引号：引号由渲染时按语言补，故夹具里存的是原始关键词
    expect(rows[0].text()).toContain("“workspace/SeshBuddy” · “sessions”");
    expect(rows[0].text()).toContain("×2");
    expect(rows[1].text()).toContain("正在提炼周报核心要点");
  });

  it("当处于未收起的思考中状态时展示实时动态计时与微光动画", async () => {
    const activeProcess: ChatMessage = {
      role: "assistant",
      kind: "process",
      text: "",
      collapsed: false,
      startedAt: Date.now() - 2500, // 2.5s 前开始
      steps: [
        { kind: "tool", actionId: "list", done: false },
      ],
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [activeProcess],
        activity: "assistant.activity.thinking",
        profile: "test-profile",
      },
    });

    const pill = wrapper.find(".thinking-pill");
    expect(pill.classes()).toContain("active");
    expect(wrapper.find(".shimmer-text").text()).toBe(t("assistant.messageList.thinking"));
    const timer = wrapper.find(".thinking-live-timer");
    expect(timer.exists()).toBe(true);
    expect(timer.text()).toMatch(/\d+(\.\d+)?s/);
  });

  it("活动提示随语言切换（activity 装的是语言包键，不是渲染结果）", async () => {
    const wrapper = mount(ChatMessageList, {
      props: { messages: [], activity: "assistant.activity.thinking", profile: "test-profile" },
    });

    try {
      expect(wrapper.find(".activity-text").text()).toBe("Thinking…");
      setLocale("zh");
      await nextTick();
      // 这一段是整段等待期的常驻提示：若 activity 存的是渲染结果，切语言后它不会跟着变
      expect(wrapper.find(".activity-text").text()).toBe("正在思考…");
    } finally {
      setLocale("en");
      await nextTick();
    }
  });

  it("复制失败时不显示「已复制」，成功时才显示", async () => {
    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [{ role: "assistant", kind: "text", text: "一段回答" } as ChatMessage],
        activity: null,
        profile: "test-profile",
      },
    });
    const btn = () => wrapper.find(".copy-icon-btn");

    // 三层链路全失败：原生抛错、Web 层被拒、第三层（jsdom 里没有 `document.execCommand`，
    // 调用即抛）由 `copyToClipboard` 的 catch 兜住并返回 false
    clipboardWrite.writeText.mockRejectedValue(new Error("denied"));
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });

    await btn().trigger("click");
    await flushPromises();
    // 旧实现无条件设 `copiedIndex`，写入被拒时界面照样打勾——这条钉住它
    expect(wrapper.find(".copied-check").exists()).toBe(false);

    // 阳性对照：同样的调用路径，写入成功时**必须**显示。
    // 少了它，上面那条会因为「恒不显示」而通过，判据就没有区分力。
    clipboardWrite.writeText.mockResolvedValue(undefined);
    await btn().trigger("click");
    await flushPromises();
    expect(wrapper.find(".copied-check").exists()).toBe(true);
  });

  it("渲染常规 Assistant 文本回复与复制按钮", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "这是 SeshBuddy 助手的回答内容",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    expect(wrapper.find(".assistant-row").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").text()).toContain("这是 SeshBuddy 助手的回答内容");
    expect(wrapper.find(".copy-icon-btn").exists()).toBe(true);
  });

  it("语言切换后代码块复制按钮的文案跟随语言", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "```js\nconst a = 1;\n```",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
      },
    });

    // 复制按钮由 enhanceCodeBlocks 插进 v-html 内容，不在 Vue 渲染树里
    expect(wrapper.find(".code-copy-btn").text()).toBe(t("app.ctxMenu.copy"));

    try {
      setLocale("zh");
      await nextTick();
      expect(wrapper.find(".code-copy-btn").text()).toBe("复制");
    } finally {
      setLocale("en");
      await nextTick();
    }
  });

  it("语言切换后已存在的步骤文案跟随语言（不冻结在产生时的语言）", async () => {
    const processMessage: ChatMessage = {
      role: "assistant",
      kind: "process",
      text: "",
      collapsed: false,
      startedAt: Date.now(),
      steps: [
        { kind: "tool", actionId: "read", done: false },
        { kind: "tool", actionId: "search", done: false, targets: ["认证失败"] },
      ],
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [processMessage, { role: "assistant", kind: "text", text: "答案" }],
        activity: null,
      },
    });

    const chips = wrapper.findAll(".chip-action-name");
    expect(chips[0].text()).toBe("Read conversation text");
    expect(wrapper.find(".chip-target-pill").text()).toBe("“认证失败”");

    try {
      setLocale("zh");
      await nextTick();
      // 复制按钮的 title 由 t() 现取：它切到中文即证明这次渲染副作用确实重跑了
      expect(wrapper.find(".copy-icon-btn").attributes("title")).toBe("复制 Markdown 全文");
      // 而步骤文案若不跟随语言，就是被冻结在产生时的语言里
      expect(chips[0].text()).toBe("读取会话正文");
      expect(chips[1].text()).toBe("搜索会话内容");
      expect(wrapper.find(".chip-target-pill").text()).toBe("「认证失败」");
    } finally {
      setLocale("en");
      await nextTick();
    }
  });

  it("渲染 09 Recommendation 原语：当模型输出 «FOLLOWUPS: ...» 时动态渲染追问药丸，且正文不出现标签", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "本周完成了关于 SeshBuddy 助手 Thinking 和 Tool Chips 的功能重构。\n«FOLLOWUPS: 按重点与难点细化周报 | 补充单元测试用例»",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    const followups = wrapper.findAll(".followup-pill");
    expect(followups.length).toBe(2);
    expect(followups[0].text()).toContain("按重点与难点细化周报");
    expect(followups[1].text()).toContain("补充单元测试用例");

    // Markdown 正文中被干净剥离，不暴露给用户
    expect(wrapper.find(".markdown-body").text()).not.toContain("«FOLLOWUPS:");
    expect(wrapper.find(".markdown-body").text()).toContain("本周完成了关于 SeshBuddy 助手");

    await followups[0].trigger("click");
    expect(wrapper.emitted("pick")?.[0]).toEqual(["按重点与难点细化周报"]);
  });

  it("当模型未输出 FOLLOWUPS 时不渲染追问药丸（无写死规则）", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "这是一条没有附带追问的普通回复内容",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    expect(wrapper.findAll(".followup-pill")).toHaveLength(0);
  });

  it("用户发送新消息时，即使此前上翻暂停了跟随，也会恢复 follow 为 true 并滚动到底部", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "历史回复",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    // 模拟容器尺寸与滚动高度
    Object.defineProperty(listEl, "scrollHeight", { value: 1000, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 200; // 上翻距离底部 > 40px

    // 触发上翻滚动事件
    await wrapper.find(".chat-list").trigger("scroll");
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);

    // 用户发出新消息
    const userMessage: ChatMessage = {
      role: "user",
      kind: "text",
      text: "用户新发送的问题",
    };

    await wrapper.setProps({
      messages: [assistantMessage, userMessage],
    });

    // 应该恢复跟随，并将 scrollTop 置为 scrollHeight
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(true);
    expect(listEl.scrollTop).toBe(1000);
  });

  it("用户上翻后助手流式更新文本时，不打断用户阅读（不强制置底）", async () => {
    const userMessage: ChatMessage = {
      role: "user",
      kind: "text",
      text: "请总结",
    };
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "正在",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [userMessage, assistantMessage],
        activity: "正在思考…",
        profile: "test-profile",
      },
    });
    await new Promise((r) => setTimeout(r, 50));

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    Object.defineProperty(listEl, "scrollHeight", { value: 1000, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 100; // 用户上翻阅读

    await wrapper.find(".chat-list").trigger("scroll");
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);

    // 助手继续流式追加文本（消息数量未增加，仅文本增长）
    await wrapper.setProps({
      messages: [userMessage, { ...assistantMessage, text: "正在输出更多内容…" }],
    });

    // follow 依然保持 false，且不强行滚动到底部
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);
    expect(listEl.scrollTop).toBe(100);
  });

  it("调用 scrollToBottom(true) 暴露方法直接重置 follow 并滚动到底部", async () => {
    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [{ role: "user", kind: "text", text: "测试" }],
        activity: null,
      },
    });

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    Object.defineProperty(listEl, "scrollHeight", { value: 1200, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 100;

    await wrapper.find(".chat-list").trigger("scroll");
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);

    await (wrapper.vm as unknown as { scrollToBottom: (force?: boolean) => Promise<void> }).scrollToBottom(true);
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(true);
    expect(listEl.scrollTop).toBe(1200);
  });

  it("用户上翻浏览期间，新消息到达或流式输出时累计 unreadCount，并在点击置底按钮后恢复跟随", async () => {
    const userMessage: ChatMessage = {
      role: "user",
      kind: "text",
      text: "请解释代码",
    };
    const assistantMessage1: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "第一条消息",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [userMessage, assistantMessage1],
        activity: null,
        profile: "test-profile",
      },
    });

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    Object.defineProperty(listEl, "scrollHeight", { value: 1500, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 200; // 上翻离开底部

    await wrapper.find(".chat-list").trigger("scroll");
    const vm = wrapper.vm as unknown as {
      follow: boolean;
      unreadCount: number;
      scrollToBottom: (force?: boolean) => Promise<void>;
    };
    expect(vm.follow).toBe(false);
    expect(vm.unreadCount).toBe(0);

    // 助手追加新消息 2
    const assistantMessage2: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "第二条消息",
    };
    await wrapper.setProps({
      messages: [userMessage, assistantMessage1, assistantMessage2],
    });

    // 此时未读计数应为 1
    expect(vm.unreadCount).toBe(1);
    expect(vm.follow).toBe(false);

    // 界面上渲染了带有未读计数的悬浮胶囊
    expect(wrapper.find(".unread-text").text()).toBe(t("common.scrollToBottom.newMessages", { count: 1 }));

    // 助手继续追加新消息 3
    const assistantMessage3: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "第三条消息",
    };
    await wrapper.setProps({
      messages: [userMessage, assistantMessage1, assistantMessage2, assistantMessage3],
    });
    expect(vm.unreadCount).toBe(2);
    expect(wrapper.find(".unread-text").text()).toBe(t("common.scrollToBottom.newMessages", { count: 2 }));

    // 点击置底按钮触发跳转
    const button = wrapper.find(".scroll-btn");
    expect(button.exists()).toBe(true);
    await button.trigger("click");

    // 应重置未读计数、恢复跟随状态
    expect(vm.unreadCount).toBe(0);
    expect(vm.follow).toBe(true);
  });

  it("用户上翻期间思考过程流式更新时累积未读提示，用户手动滚到底部后自动清除", async () => {
    const userMessage: ChatMessage = {
      role: "user",
      kind: "text",
      text: "帮我查询",
    };
    const processMessage: ChatMessage = {
      role: "assistant",
      kind: "process",
      text: "",
      collapsed: false,
      steps: [{ kind: "tool", actionId: "list", done: false }],
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [userMessage, processMessage],
        activity: "正在查询...",
        profile: "test-profile",
      },
    });

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    Object.defineProperty(listEl, "scrollHeight", { value: 1200, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 300;

    await wrapper.find(".chat-list").trigger("scroll");
    const vm = wrapper.vm as unknown as { follow: boolean; unreadCount: number };
    expect(vm.follow).toBe(false);
    expect(vm.unreadCount).toBe(0);

    // 步骤增加
    await wrapper.setProps({
      messages: [
        userMessage,
        {
          ...processMessage,
          steps: [
            { kind: "tool", actionId: "list", done: true },
            { kind: "tool", actionId: "read", done: false },
          ],
        },
      ],
    });

    expect(vm.unreadCount).toBe(1);
    expect(wrapper.find(".unread-text").text()).toBe(t("common.scrollToBottom.newMessages", { count: 1 }));

    // 用户手动滚到底部（距离底部 < 40px）
    listEl.scrollTop = 1200 - 400 - 20; // 780
    await wrapper.find(".chat-list").trigger("scroll");

    expect(vm.follow).toBe(true);
    expect(vm.unreadCount).toBe(0);
  });

  it("空会话的引用胶囊 tooltip 落到本地化兜底句（后端 title 为 null）", async () => {
    // 后端对「标题与首条消息都为空」的会话返回 title: null
    vi.mocked(invoke).mockResolvedValue([
      { title: null, filePath: "/tmp/empty.jsonl", cliId: "claude" },
    ]);

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [{ role: "assistant", kind: "text", text: "见 ⟦1:aaaaaaaa⟧" }],
        activity: null,
        profile: "test-profile",
      },
    });
    await flushPromises();

    const badge = wrapper.find(".session-ref");
    expect(badge.exists()).toBe(true);
    // 兜底句复用归档会话那条既有键（同一句「这个会话没有标题」），不新造键
    expect(badge.attributes("title")).toBe(t("settings.archivedSessions.untitledSession"));
    // 拿得到胶囊即证明 renderBody 确实做了归一：没有它，null 会走到 renderSessionRefs
    // 的 `== null` 防御判据上被当成无效引用抹掉，这里就没有胶囊可查。
    expect(wrapper.find(".markdown-body").html()).not.toContain('title="null"');

    try {
      setLocale("zh");
      await nextTick();
      expect(wrapper.find(".session-ref").attributes("title")).toBe("未命名会话");
    } finally {
      setLocale("en");
      await nextTick();
    }
  });

  it("（复位自证）上一个用例的 title: null 桩不泄漏到本用例", async () => {
    // beforeEach 把 invoke 复位成默认的 `[]`：引用解析返回空数组 → 引用按无效抹掉。
    // 若上一个用例的桩泄漏过来，这里要么渲染出胶囊、要么抛 TypeError，本用例会红。
    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [{ role: "assistant", kind: "text", text: "见 ⟦1:aaaaaaaa⟧" }],
        activity: null,
        profile: "test-profile",
      },
    });
    await flushPromises();

    expect(wrapper.find(".session-ref").exists()).toBe(false);
    expect(wrapper.find(".markdown-body").text().trim()).toBe("见");
  });
});

