import { describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { reduceAssistantEvent, activityFromToolUse, stepFromToolUse, stepActionLabel, buildHistoryMessages, humanizeError, useAssistantChat, type ActivityAction } from "./useAssistantChat";
import { setLocale } from "../i18n";

// 通道 3 的消费点：`assistant-event` 载荷里的 `error` 字段。
// 监听器只能在 mock 掉 listen/invoke 之后驱动，故这两个模块必须换掉。
const invokeMock = vi.fn();
let eventHandler: ((ev: { payload: unknown }) => void) | null = null;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (_event: string, handler: (ev: { payload: unknown }) => void) => {
    eventHandler = handler;
    return Promise.resolve(() => {
      eventHandler = null;
    });
  },
}));

describe("reduceAssistantEvent", () => {
  it("error 事件生成 error kind 气泡", () => {
    const msgs = reduceAssistantEvent([], { type: "error", text: "CLI 未找到" });
    expect(msgs).toEqual([{ role: "assistant", kind: "error", text: "CLI 未找到" }]);
  });

  it("assistant-text 流式追加到最后一条文本气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "你" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "好" });
    expect(msgs).toEqual([{ role: "assistant", kind: "text", text: "你好" }]);
  });

  it("user 事件追加用户气泡", () => {
    const msgs = reduceAssistantEvent([], { type: "user", text: "问题" });
    expect(msgs[0]).toEqual({ role: "user", kind: "text", text: "问题" });
  });

  it("reducer 不变更输入数组（immutability）", () => {
    const msgs = [{ role: "assistant", kind: "text", text: "已有" } as const];
    const after = reduceAssistantEvent(msgs, { type: "assistant-text", text: "追加" });
    expect(after).not.toBe(msgs);
    expect(msgs).toHaveLength(1);
    expect(msgs[0].text).toBe("已有");
  });

  it("error 气泡后接 assistant-text 新开气泡（不追加进 error 气泡）", () => {
    let msgs = reduceAssistantEvent([], { type: "error", text: "出错了" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "重新回答" });
    expect(msgs).toEqual([
      { role: "assistant", kind: "error", text: "出错了" },
      { role: "assistant", kind: "text", text: "重新回答" },
    ]);
  });
});

describe("activityFromToolUse", () => {
  it("按 proxy 子命令映射人话动作", () => {
    expect(activityFromToolUse("Bash", "seshbuddy-proxy list --days 7")).toBe("Listing conversations…");
    expect(activityFromToolUse("Bash", "seshbuddy-proxy grep 登录")).toBe("Searching conversation content…");
    expect(activityFromToolUse("Bash", "seshbuddy-proxy show abc --from 1")).toBe("Reading conversation text…");
  });
  it("未知命令给通用文案", () => {
    expect(activityFromToolUse("Bash", "ls")).toBe("Running command…");
    expect(activityFromToolUse(undefined, undefined)).toBe("Running command…");
  });
});

describe("stepFromToolUse（步骤动作与目标）", () => {
  it("show 提取会话 id 前缀", () => {
    expect(stepFromToolUse("seshbuddy-proxy show 3f8e5341-31e2-41cd-a178 --from 1 --to 30")).toEqual(
      { actionId: "read", target: "3f8e5341" },
    );
  });
  it("grep 提取关键词（目标存原始关键词，引号由渲染时按语言补）", () => {
    expect(stepFromToolUse('seshbuddy-proxy grep "认证失败" --days 30')).toEqual({
      actionId: "search", target: "认证失败",
    });
    expect(stepFromToolUse("seshbuddy-proxy grep 登录")).toEqual({
      actionId: "search", target: "登录",
    });
  });
  it("list 与未知命令无目标", () => {
    expect(stepFromToolUse("seshbuddy-proxy list --days 7")).toEqual({ actionId: "list", target: null });
    expect(stepFromToolUse("ls")).toEqual({ actionId: "run", target: null });
    expect(stepFromToolUse(undefined)).toEqual({ actionId: "run", target: null });
  });
});

describe("同动作步骤聚合", () => {
  it("连续同动作步骤聚合并收集目标（不同目标也聚合）", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy show 3f8e5341-abc --from 1" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show 29d9f42d-def" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show 3f8e5341-abc --from 31" });
    expect(msgs[0].steps).toEqual([
      { kind: "tool", actionId: "read", done: false, count: 3, targets: ["3f8e5341", "29d9f42d"] },
    ]);
  });

  it("解说打断后同动作另起新步骤", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy show 3f8e5341-abc" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "再深入读一遍" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show 29d9f42d-def" });
    // 解说被后续工具吸收为 note；新工具步骤因 note 隔开不并入旧步骤
    expect(msgs[0].steps).toHaveLength(3);
    expect(msgs[0].steps?.[0]).toMatchObject({ actionId: "read", targets: ["3f8e5341"] });
    expect(msgs[0].steps?.[1]).toMatchObject({ kind: "note" });
    expect(msgs[0].steps?.[2]).toMatchObject({ actionId: "read", targets: ["29d9f42d"] });
  });

  it("不同动作不聚合", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy show 3f8e5341-abc" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy list" });
    expect(msgs[0].steps).toHaveLength(2);
  });

  it("摘要步数按聚合后的调用总数计算", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy show 3f8e5341-abc --from 1" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show 29d9f42d-def" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    const tools = (msgs[0].steps ?? []).filter((s) => s.kind === "tool");
    const totalCalls = tools.reduce((n, s) => n + (s.count ?? 1), 0);
    expect(totalCalls).toBe(3);
  });
});

describe("步骤与语言无关", () => {
  // 聚合判据若比对「已解析文案」，同一动作在语言切换前后就不再相等，一条步骤会裂成两条。
  // 这里刻意不断言任何文案，只断言聚合结果——它在改数据形态前后都成立。
  it("同一动作跨语言切换仍聚合为一条步骤", () => {
    setLocale("en");
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy show 3f8e5341-abc" });
    setLocale("zh");
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show 29d9f42d-def" });
    setLocale("en");
    expect(msgs[0].steps).toHaveLength(1);
    expect(msgs[0].steps?.[0]).toMatchObject({
      kind: "tool",
      count: 2,
      targets: ["3f8e5341", "29d9f42d"],
    });
  });

  // 动作标识 → 语言包键的映射写在 ACTIVITY_KEYS 里，不再是 t("字面量")，
  // 闸门规则 5 因此看不见这四个 key。这条用例补上那份「key 还在不在」的保证。
  // 语言自己钉住：不依赖上一条用例末尾的复位，否则它一旦提前失败就会连累这里报错报错对象。
  it("四个动作标识都取得到文案", () => {
    setLocale("en");
    const label = (actionId: ActivityAction) => stepActionLabel({ kind: "tool", actionId, done: false });
    expect(label("list")).toBe("List conversations");
    expect(label("search")).toBe("Search conversation content");
    expect(label("read")).toBe("Read conversation text");
    expect(label("run")).toBe("Run command");
  });
});

describe("过程卡片（process block）", () => {
  it("首个 tool-use 开启过程卡片，步骤为进行中", () => {
    const msgs = reduceAssistantEvent([], {
      type: "tool-use",
      summary: "seshbuddy-proxy list --days 7",
    });
    expect(msgs).toEqual([
      {
        role: "assistant",
        kind: "process",
        text: "",
        collapsed: false,
        startedAt: expect.any(Number),
        steps: [{ kind: "tool", actionId: "list", done: false }],
      },
    ]);
  });

  it("连续 tool-use 追加步骤，前序步骤标记完成", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show abc" });
    expect(msgs).toHaveLength(1);
    expect(msgs[0].steps).toEqual([
      { kind: "tool", actionId: "list", done: true },
      { kind: "tool", actionId: "read", done: false },
    ]);
  });

  it("assistant-text 到来时步骤全部标记完成，块保持敞开，文本另起气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "查到了" });
    expect(msgs).toHaveLength(2);
    expect(msgs[0].kind).toBe("process");
    expect(msgs[0].collapsed).toBe(false);
    expect(msgs[0].steps).toEqual([{ kind: "tool", actionId: "list", done: true }]);
    expect(msgs[1]).toMatchObject({ kind: "text", text: "查到了" });
  });

  it("被工具调用打断的助手文本是过程解说，吸收进过程卡片", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "先读几个重点会话的开头" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy show abc" });
    expect(msgs).toHaveLength(1);
    expect(msgs[0].kind).toBe("process");
    expect(msgs[0].steps).toEqual([
      { text: "先读几个重点会话的开头", done: true, kind: "note" },
      { kind: "tool", actionId: "read", done: false },
    ]);
  });

  it("一轮结束：解说进卡片，只有最终答案留作气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "先过滤元查询会话" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "最终答案" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end", durationMs: 1000 });
    expect(msgs).toHaveLength(2);
    expect(msgs[0].collapsed).toBe(true);
    expect(msgs[1]).toMatchObject({ kind: "text", text: "最终答案" });
  });

  it("turn-end 收起卡片并记录耗时", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "答案" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end", durationMs: 32000 });
    const block = msgs[0];
    expect(block.collapsed).toBe(true);
    expect(block.durationMs).toBe(32000);
    expect(block.steps?.every((s) => s.done)).toBe(true);
  });

  it("收起后再来 tool-use 开启新卡片（多轮不串）", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "seshbuddy-proxy grep 登录" });
    expect(msgs).toHaveLength(2);
    expect(msgs[0].collapsed).toBe(true);
    expect(msgs[1]).toMatchObject({ kind: "process", collapsed: false });
    expect(msgs[1].steps).toEqual([{ kind: "tool", actionId: "search", done: false, targets: ["登录"] }]);
  });

  it("error 事件收起敞开的过程卡片再上屏错误", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "seshbuddy-proxy show x" });
    msgs = reduceAssistantEvent(msgs, { type: "error", text: "挂了" });
    expect(msgs[0].collapsed).toBe(true);
    expect(msgs[1]).toMatchObject({ kind: "error", text: "挂了" });
  });

  it("无过程卡片时 turn-end 不产生任何变化", () => {
    const before = reduceAssistantEvent([], { type: "assistant-text", text: "纯文本" });
    const after = reduceAssistantEvent(before, { type: "turn-end" });
    expect(after).toEqual(before);
  });
});

describe("buildHistoryMessages（历史回放）", () => {
  it("同 turn 非末尾 assistant 段收成解说卡片，末尾段留作答案", () => {
    const msgs = buildHistoryMessages([
      { role: "user", text: "生成周报" },
      { role: "assistant", text: "先过滤元查询会话" },
      { role: "assistant", text: "现在可以生成周报了" },
      { role: "assistant", text: "# 周报正文" },
    ]);
    expect(msgs).toHaveLength(3);
    expect(msgs[0]).toMatchObject({ role: "user", kind: "text" });
    expect(msgs[1]).toMatchObject({ kind: "process", collapsed: true });
    expect(msgs[1].steps).toEqual([
      { text: "先过滤元查询会话", done: true, kind: "note" },
      { text: "现在可以生成周报了", done: true, kind: "note" },
    ]);
    expect(msgs[2]).toMatchObject({ kind: "text", text: "# 周报正文" });
  });

  it("单段 turn 不产生过程卡片", () => {
    const msgs = buildHistoryMessages([
      { role: "user", text: "你好" },
      { role: "assistant", text: "你好！" },
    ]);
    expect(msgs).toEqual([
      { role: "user", kind: "text", text: "你好" },
      { role: "assistant", kind: "text", text: "你好！" },
    ]);
  });

  it("过滤非 user/assistant 角色", () => {
    const msgs = buildHistoryMessages([
      { role: "system", text: "x" },
      { role: "user", text: "问题" },
      { role: "assistant", text: "回答" },
    ]);
    expect(msgs).toHaveLength(2);
  });
});

describe("turn-end usage 挂载", () => {
  it("turn-end 带 usage 时挂到最后一条 assistant 文本气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "回答" });
    msgs = reduceAssistantEvent(msgs, {
      type: "turn-end",
      durationMs: 1200,
      usage: { inputTokens: 100, outputTokens: 20, cacheReadInputTokens: 0, cacheCreationInputTokens: 0 },
    });
    expect(msgs[0].usage).toEqual({
      inputTokens: 100, outputTokens: 20, cacheReadInputTokens: 0, cacheCreationInputTokens: 0,
    });
  });

  it("turn-end 无 usage 时消息不受影响", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "回答" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    expect(msgs[0].usage).toBeUndefined();
    expect(msgs[0].text).toBe("回答");
  });
});

describe("cancelled 事件", () => {
  it("给最后一条 assistant 文本打 stopped 标记并保留内容", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "已生成一半" });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[0]).toMatchObject({ kind: "text", text: "已生成一半", stopped: true });
  });

  it("收起敞开的过程卡片", () => {
    let msgs = reduceAssistantEvent([], {
      type: "tool-use", name: "Bash", summary: "seshbuddy-proxy list --days 7",
    });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[0].kind).toBe("process");
    expect(msgs[0].collapsed).toBe(true);
  });

  it("没有 assistant 文本时原样返回（不报错）", () => {
    const msgs = reduceAssistantEvent([], { type: "user", text: "问" });
    const after = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(after).toHaveLength(1);
    expect(after[0].stopped).toBeUndefined();
  });

  it("新一轮未产出文本时取消，不标记上一轮的旧消息", () => {
    // 回归：停止「你可以干嘛」时标记曾错误落在上一轮「你是谁啊」的回答上
    let msgs = reduceAssistantEvent([], { type: "user", text: "第一问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第一答" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    msgs = reduceAssistantEvent(msgs, { type: "user", text: "第二问" });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[1].stopped).toBeUndefined(); // 上一轮回答不被误标
    expect(msgs).toHaveLength(3);
  });

  it("新一轮已有文本时取消，标记本轮消息而非旧消息", () => {
    let msgs = reduceAssistantEvent([], { type: "user", text: "第一问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第一答" });
    msgs = reduceAssistantEvent(msgs, { type: "user", text: "第二问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第二答一半" });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[1].stopped).toBeUndefined();
    expect(msgs[3]).toMatchObject({ text: "第二答一半", stopped: true });
  });

  it("turn-end 的 usage 同样只挂本轮消息", () => {
    let msgs = reduceAssistantEvent([], { type: "user", text: "第一问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第一答" });
    msgs = reduceAssistantEvent(msgs, { type: "user", text: "第二问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第二答" });
    msgs = reduceAssistantEvent(msgs, {
      type: "turn-end",
      usage: { inputTokens: 10, outputTokens: 5, cacheReadInputTokens: 0, cacheCreationInputTokens: 0 },
    });
    expect(msgs[1].usage).toBeUndefined();
    expect(msgs[3].usage?.inputTokens).toBe(10);
  });
});

describe("buildHistoryMessages usage 回填", () => {
  it("assistant 段的 usage 挂到最终答案气泡（snake 转 camel）", () => {
    const msgs = buildHistoryMessages([
      { role: "user", text: "问" },
      { role: "assistant", text: "答", usage: { input_tokens: 100, output_tokens: 20 } },
    ]);
    expect(msgs[1].usage).toEqual({
      inputTokens: 100, outputTokens: 20, cacheReadInputTokens: 0, cacheCreationInputTokens: 0,
    });
  });

  it("同 turn 多段时取最后一段的 usage，前段解说进过程卡片不带 usage", () => {
    const msgs = buildHistoryMessages([
      { role: "assistant", text: "先查一下", usage: { input_tokens: 50, output_tokens: 10 } },
      { role: "assistant", text: "最终答案", usage: { input_tokens: 200, output_tokens: 30 } },
    ]);
    const answer = msgs.find((m) => m.kind === "text");
    expect(answer?.text).toBe("最终答案");
    expect(answer?.usage?.inputTokens).toBe(200);
  });

  it("usage 为 null/缺失时不挂字段", () => {
    const msgs = buildHistoryMessages([
      { role: "assistant", text: "答", usage: null },
      { role: "assistant", text: "答2" },
    ]);
    const answer = msgs.find((m) => m.kind === "text");
    expect(answer?.usage).toBeUndefined();
  });
});

// 这两个取值是**跨进程协议状态**（「CLI 没装」「上一轮没跑完」），不是界面文案：
// 后端在 src-tauri/src/assistant/commands.rs 里以 AppError::Coded 发 code，humanizeError
// 按 code 换成专门提示、不认就把原文直接上屏。早先的做法是按后端中文原文做子串匹配，
// 语言包里为此存了一份四语刻意不译的副本——译文一变（哪怕只改一门）分支就永不命中，
// 专门提示静默退化成原始错误串，且没有别的用例覆盖命中分支。
//
// 下面两条是这层耦合的绊线，缺一环就漏一类退化：
// ① 钉后端仍在发出这两个 code；② 钉代码仍在按它们匹配。
// 只钉①：把 humanizeError 里那两个 if 分支整个删掉，①照样全绿——
// 而那种退化用户看得见（专门提示消失），测试却看不见。①+② 才等价于「仍然命中」。
const BACKEND_CLI_MISSING = "assistant.cli_missing"; // commands.rs 的 AppError::coded
const BACKEND_TURN_IN_PROGRESS = "assistant.turn_in_progress"; // commands.rs 的 AppError::coded
// 不写成 `new URL("../../…", import.meta.url)`：那个字面形状是 Vite 的资源引用语法，
// 会被改写成非 file: 的模块 URL，fileURLToPath 直接抛「The URL must be of scheme file」。
// 经一个变量转手可避开该改写，实测取到的是本文件真实路径。
const TEST_FILE_URL = import.meta.url;
const BACKEND_COMMANDS_RS = join(
  fileURLToPath(new URL("../../", TEST_FILE_URL)),
  "src-tauri/src/assistant/commands.rs",
);

describe("assistant 的两条协议标记", () => {
  it("后端仍在发出这两个 code（改后端也会红）", () => {
    const rust = readFileSync(BACKEND_COMMANDS_RS, "utf8");
    for (const code of [BACKEND_CLI_MISSING, BACKEND_TURN_IN_PROGRESS]) {
      expect(rust, `${BACKEND_COMMANDS_RS} 里已找不到 coded("${code}")`).toContain(
        `coded("${code}")`,
      );
    }
  });

  // 两条分支各覆盖一次，喂的是**原始载荷形状**（`{ code, params }`，与事件通道一致）：
  // 判据若退回「从渲染好的文案里找子串」，这两条会因拿到对象而直接红。
  it("humanizeError 仍按 code 匹配（删掉那两个 if 分支就会红）", () => {
    setLocale("en");
    expect(humanizeError({ code: BACKEND_CLI_MISSING, params: {} })).toBe(
      "Claude CLI not detected. Install and configure it first.",
    );
    expect(humanizeError({ code: BACKEND_TURN_IN_PROGRESS })).toBe(
      "The previous reply is still in progress. Please wait.",
    );
    // 反向一：别的错误必须原样透传，否则专门提示会吞掉所有别的错误原文。
    expect(humanizeError("完全是别的错误")).toBe("完全是别的错误");
    // 反向二：只有旧文案没有 code 不再命中——耦合被拆掉的证据（旧实现下它会红）。
    expect(humanizeError("未检测到 claude CLI: boom")).toBe("未检测到 claude CLI: boom");
  });
});

describe("assistant-event 的 error 字段（通道 3）", () => {
  /** 起一轮对话，返回已注册好监听的 chat 实例。 */
  async function startTurn() {
    const chat = useAssistantChat();
    invokeMock.mockResolvedValueOnce({ conversationId: "c1" });
    await chat.send("你好", "default");
    return chat;
  }

  function emit(payload: unknown) {
    if (!eventHandler) throw new Error("assistant-event 的监听器尚未注册");
    eventHandler({ payload });
  }

  function lastMessage(chat: ReturnType<typeof useAssistantChat>) {
    return chat.messages.value[chat.messages.value.length - 1];
  }

  // 与通道 2 同一条判据：`{ code, params }` 与裸字符串两种形状都要有控制。
  it("error 载荷是 { code, params } 时渲染当前语言的语言包文案", async () => {
    const chat = await startTurn();
    emit({
      conversationId: "c1",
      type: "error",
      error: { code: "assistant.agent_failed", params: { detail: "boom" } },
    });
    expect(lastMessage(chat)).toMatchObject({ kind: "error", text: "Agent run failed: boom" });
  });

  it("error 载荷是字符串时原样上屏（R1 迁移期形状）", async () => {
    const chat = await startTurn();
    emit({ conversationId: "c1", type: "error", error: "CLI 未找到" });
    expect(lastMessage(chat)).toMatchObject({ kind: "error", text: "CLI 未找到" });
  });

  // result 事件的 `error` 由 CLI 自己的 result 行填充（agent.rs 从流里解析）。
  // 线上形状是 `{code, params}`：CLI 原文按 R3 放在 `detail` 里不翻译，包一层码只为给出
  // 出处。这里断言**渲染后**的文案——还去读旧的裸字符串形状会立刻失败。
  it("result 事件的 error 字段是 { code, params } 时渲染出带出处的文案", async () => {
    const chat = await startTurn();
    emit({
      conversationId: "c1",
      event: {
        type: "result",
        ok: false,
        error: { code: "assistant.cli_error", params: { detail: "API Error: 400 bad model" } },
      },
    });
    // CLI 原文逐字保留在句尾（未翻译），前面是当前语言的出处说明
    expect(lastMessage(chat)).toMatchObject({
      kind: "error",
      text: "The CLI returned an error: API Error: 400 bad model",
    });
  });

  // 反向控制：`Business` 变体仍是裸字符串（R1），原样上屏、不查语言包。
  it("result 事件的 error 字段是字符串时原样上屏", async () => {
    const chat = await startTurn();
    emit({ conversationId: "c1", event: { type: "result", ok: false, error: "引擎炸了" } });
    expect(lastMessage(chat)).toMatchObject({ kind: "error", text: "引擎炸了" });
  });
});
