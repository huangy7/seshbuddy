import { describe, it, expect } from "vitest";
import { ref } from "vue";
import { useMessageTurns } from "./useMessageTurns";
import type { ChatMessage } from "../types/session";

describe("useMessageTurns", () => {
  it("aggregates raw messages into per-message turns and tool groups", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-27T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "Hello" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:05Z",
        model: "anthropic.glm-5.2",
        content_parts: [
          { type: "tool_use", summary: "git diff", tool_name: "Bash", input: "git diff" },
        ],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:06Z",
        model: "anthropic.glm-5.2",
        content_parts: [
          { type: "tool_use", summary: "test-env.json", tool_name: "Read", input: "test-env.json" },
        ],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(3);
    expect(turns.value[0].userMessage?.msg.content_parts[0]).toEqual({ type: "text", text: "Hello" });
    expect(turns.value[0].assistantTurn).toBeUndefined();

    expect(turns.value[1].assistantTurn?.segments.length).toBe(1);
    const seg1 = turns.value[1].assistantTurn?.segments[0];
    expect(seg1?.type).toBe("tool_group");
    if (seg1?.type === "tool_group") {
      expect(seg1.groups.length).toBe(1);
      expect(seg1.groups[0].tool_name).toBe("Bash");
    }

    expect(turns.value[2].assistantTurn?.segments.length).toBe(1);
    const seg2 = turns.value[2].assistantTurn?.segments[0];
    expect(seg2?.type).toBe("tool_group");
    if (seg2?.type === "tool_group") {
      expect(seg2.groups.length).toBe(1);
      expect(seg2.groups[0].tool_name).toBe("Read");
    }
  });

  it("merges parts within a single message, keeps them one turn", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-27T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "Explain this code" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:01Z",
        model: "anthropic.glm-5.2",
        content_parts: [
          { type: "thinking", thinking: "Thinking part 1. " },
          { type: "thinking", thinking: "Thinking part 2." },
          { type: "text", text: "First text line. " },
          { type: "text", text: "Second text line." },
          { type: "tool_use", summary: "read file", tool_name: "Read", input: "file.ts" },
          { type: "text", text: "After tool text." },
        ],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    // 用户一条 + 助手一条 = 两个独立回合；同一条消息内的多段文本/思考合并
    expect(turns.value.length).toBe(2);
    expect(turns.value[0].userMessage).toBeDefined();
    expect(turns.value[0].assistantTurn).toBeUndefined();

    const segments = turns.value[1].assistantTurn?.segments || [];
    expect(segments.length).toBe(4);

    expect(segments[0].type).toBe("thinking");
    if (segments[0].type === "thinking") {
      expect(segments[0].thinking).toBe("Thinking part 1. Thinking part 2.");
    }

    expect(segments[1].type).toBe("text");
    if (segments[1].type === "text") {
      expect(segments[1].text).toBe("First text line. Second text line.");
    }

    expect(segments[2].type).toBe("tool_group");

    expect(segments[3].type).toBe("text");
    if (segments[3].type === "text") {
      expect(segments[3].text).toBe("After tool text.");
    }
  });

  it("matches tool_result to tool_use tool_name by tool_use_id", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-27T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "Run git status" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:02Z",
        model: "anthropic.glm-5.2",
        content_parts: [
          {
            type: "tool_use",
            summary: "git status",
            tool_name: "Bash",
            input: "git status",
            tool_use_id: "call_abc123",
          },
          {
            type: "tool_result",
            summary: "On branch main",
            content: "On branch main\nnothing to commit",
            is_error: false,
            tool_use_id: "call_abc123",
          },
          {
            type: "tool_result",
            summary: "Unmatched result",
            content: "Unknown result",
            is_error: false,
          },
        ],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(2);
    const segments = turns.value[1].assistantTurn?.segments || [];
    expect(segments.length).toBe(1);
    expect(segments[0].type).toBe("tool_group");

    if (segments[0].type === "tool_group") {
      const groups = segments[0].groups;
      expect(groups.length).toBe(2);

      const bashGroup = groups.find((g) => g.tool_name === "Bash");
      expect(bashGroup).toBeDefined();
      expect(bashGroup?.items.length).toBe(2);
      expect(bashGroup?.items[1].tool_name).toBe("Bash");

      const resultGroup = groups.find((g) => g.tool_name === "Result");
      expect(resultGroup).toBeDefined();
      expect(resultGroup?.items.length).toBe(1);
    }
  });

  it("detects tool errors correctly via is_error, summary, or content", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-27T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "Run commands" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:05Z",
        model: "anthropic.glm-5.2",
        content_parts: [
          {
            type: "tool_result",
            summary: "Command output",
            content: "Error message",
            is_error: true,
            tool_use_id: "call_1",
          },
          {
            type: "tool_use",
            summary: "Failed execution [Tool Error]",
            tool_name: "Bash",
            input: "invalid_cmd",
          },
          {
            type: "tool_result",
            summary: "Output",
            content: "Failed with [Tool Error]: file not found",
            is_error: false,
          },
        ],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    const segment = turns.value[1].assistantTurn?.segments[0];
    expect(segment?.type).toBe("tool_group");

    if (segment?.type === "tool_group") {
      expect(segment.hasError).toBe(true);
      expect(segment.groups.every((g) => g.hasError)).toBe(true);
      segment.groups.forEach((g) => {
        g.items.forEach((item) => {
          expect(item.is_error).toBe(true);
        });
      });
    }
  });

  it("splits messages into per-message turns", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-27T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "First prompt" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:02Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "First response" }],
      },
      {
        role: "user",
        timestamp: "2026-07-27T14:41:00Z",
        model: null,
        content_parts: [{ type: "text", text: "Second prompt" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:41:02Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "Second response" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(4);

    expect(turns.value[0].userMessage?.msg.content_parts[0]).toEqual({
      type: "text",
      text: "First prompt",
    });
    expect(turns.value[1].assistantTurn?.segments[0]).toEqual({
      type: "text",
      text: "First response",
      originalIndex: 1,
      partIndex: 0,
      indexes: [1],
      textParts: [{ index: 0, text: "First response" }],
    });

    expect(turns.value[2].userMessage?.msg.content_parts[0]).toEqual({
      type: "text",
      text: "Second prompt",
    });
    expect(turns.value[3].assistantTurn?.segments[0]).toEqual({
      type: "text",
      text: "Second response",
      originalIndex: 3,
      partIndex: 0,
      indexes: [3],
      textParts: [{ index: 0, text: "Second response" }],
    });
  });

  it("does not create fake user cards for role === 'user' tool_result payload messages", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-27T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "Create PR" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-27T14:40:02Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "tool_use", tool_name: "Bash", summary: "git status", input: "git status", tool_use_id: "call-1" }],
      },
      {
        role: "user", // Anthropic API outputs role: user for tool results
        timestamp: "2026-07-27T14:40:03Z",
        model: null,
        content_parts: [{ type: "tool_result", tool_use_id: "call-1", summary: "status ok", content: "On branch master", is_error: false }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(2);
    expect(turns.value[0].userMessage?.msg.content_parts[0]).toEqual({ type: "text", text: "Create PR" });

    const seg = turns.value[1].assistantTurn?.segments[0];
    expect(seg?.type).toBe("tool_group");
    if (seg?.type === "tool_group") {
      expect(seg.groups[0].tool_name).toBe("Bash");
      expect(seg.groups[0].items.length).toBe(2); // tool_use + tool_result 折叠进同一张卡
    }
  });

  it("merges adjacent user image-only messages into one card", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-28T11:40:47Z",
        model: null,
        content_parts: [{ type: "image", media_type: "image/png", data: "base64-1" }],
      },
      {
        role: "user",
        timestamp: "2026-07-28T11:40:47Z",
        model: null,
        content_parts: [{ type: "image", media_type: "image/png", data: "base64-2" }],
      },
      {
        role: "user",
        timestamp: "2026-07-28T11:40:47Z",
        model: null,
        content_parts: [{ type: "image", media_type: "image/png", data: "base64-3" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-28T11:40:50Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "I see three images" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(2);
    const userMsg = turns.value[0].userMessage;
    expect(userMsg?.msg.content_parts.length).toBe(3);
    expect(userMsg?.msg.content_parts.every((p) => p.type === "image")).toBe(true);
    expect(turns.value[1].assistantTurn?.segments[0]).toEqual({
      type: "text",
      text: "I see three images",
      originalIndex: 3,
      partIndex: 0,
      indexes: [3],
      textParts: [{ index: 0, text: "I see three images" }],
    });
  });

  it("starts a new turn for image-only user message after an assistant turn", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-28T11:00:00Z",
        model: null,
        content_parts: [{ type: "text", text: "what is this?" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-28T11:00:05Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "It is a chart" }],
      },
      {
        role: "user",
        timestamp: "2026-07-28T11:01:00Z",
        model: null,
        content_parts: [{ type: "image", media_type: "image/png", data: "base64-x" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-28T11:01:05Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "ok got it" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(4);
    expect(turns.value[0].userMessage?.msg.content_parts[0]).toEqual({
      type: "text",
      text: "what is this?",
    });
    expect(turns.value[2].userMessage?.msg.content_parts[0]).toEqual({
      type: "image",
      media_type: "image/png",
      data: "base64-x",
    });
    expect(turns.value[3].assistantTurn?.segments[0]).toEqual({
      type: "text",
      text: "ok got it",
      originalIndex: 3,
      partIndex: 0,
      indexes: [3],
      textParts: [{ index: 0, text: "ok got it" }],
    });
  });

  it("merges user text and adjacent image message into one card", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-28T11:00:00Z",
        model: null,
        content_parts: [{ type: "text", text: "check these" }],
      },
      {
        role: "user",
        timestamp: "2026-07-28T11:00:01Z",
        model: null,
        content_parts: [{ type: "image", media_type: "image/png", data: "base64-y" }],
      },
      {
        role: "assistant",
        timestamp: "2026-07-28T11:00:05Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "done" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(2);
    expect(turns.value[0].userMessage?.msg.content_parts.length).toBe(2);
    expect(turns.value[0].userMessage?.msg.content_parts[0]).toEqual({
      type: "text",
      text: "check these",
    });
    expect(turns.value[0].userMessage?.msg.content_parts[1]).toEqual({
      type: "image",
      media_type: "image/png",
      data: "base64-y",
    });
  });

  it("handles Claude Code multi-part image prompt with is_meta payload and image_meta placeholder without empty cards", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-07-30T05:58:45Z",
        model: null,
        is_meta: false,
        content_parts: [{ type: "text", text: "Look at these issues" }],
      },
      {
        role: "user",
        timestamp: "2026-07-30T05:58:45Z",
        model: null,
        is_meta: true,
        content_parts: [
          { type: "image", media_type: "image/png", data: "base64-img1" },
          { type: "image", media_type: "image/png", data: "base64-img2" },
          { type: "text", text: "Look at these issues" },
        ],
      },
      {
        role: "user",
        timestamp: "2026-07-30T05:58:45Z",
        model: null,
        is_meta: true,
        content_parts: [
          { type: "image_meta" },
          { type: "image_meta" },
        ],
      },
      {
        role: "assistant",
        timestamp: "2026-07-30T05:58:50Z",
        model: "anthropic.glm-5.2",
        content_parts: [{ type: "text", text: "I will investigate." }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(2);
    const userParts = turns.value[0].userMessage?.msg.content_parts || [];
    expect(userParts.length).toBe(3); // text + img1 + img2
    expect(userParts[0]).toEqual({ type: "text", text: "Look at these issues" });
    expect(userParts[1]).toEqual({ type: "image", media_type: "image/png", data: "base64-img1" });
    expect(userParts[2]).toEqual({ type: "image", media_type: "image/png", data: "base64-img2" });
    expect(turns.value[1].assistantTurn?.segments[0]).toEqual({
      type: "text",
      text: "I will investigate.",
      originalIndex: 3,
      partIndex: 0,
      indexes: [3],
      textParts: [{ index: 0, text: "I will investigate." }],
    });
  });

  it("skips is_meta tool_result (local-command caveat) without phantom assistant card", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-08-07T05:45:25Z",
        model: null,
        is_meta: true,
        content_parts: [
          { type: "tool_result", summary: "[System caveat]", content: "<local-command-caveat>...</local-command-caveat>", is_error: false },
        ],
      },
      {
        role: "user",
        timestamp: "2026-08-07T05:45:25Z",
        model: null,
        is_meta: false,
        content_parts: [{ type: "text", text: "<local-command-stdout>done</local-command-stdout>" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(1);
    expect(turns.value[0].assistantTurn).toBeUndefined();
    expect(turns.value[0].userMessage?.originalIndex).toBe(1);
  });

  it("skips command-only user message whose cleaned text is empty", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-08-07T05:45:25Z",
        model: null,
        is_meta: false,
        content_parts: [
          { type: "text", text: "<command-name>/plugin</command-name>\n<command-message>plugin</command-message>\n<command-args></command-args>" },
        ],
      },
      {
        role: "user",
        timestamp: "2026-08-07T05:45:25Z",
        model: null,
        is_meta: false,
        content_parts: [{ type: "text", text: "<local-command-stdout>(no content)</local-command-stdout>" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    expect(turns.value.length).toBe(1);
    expect(turns.value[0].userMessage?.originalIndex).toBe(1);
  });

  it("records the original part index on text segments (packed DSH messages)", () => {
    // DSH 把 [thinking, text, tool_use] 打包进同一条消息；text 段渲染靠
    // (originalIndex, partIndex) 回查原始消息的 content_parts，段必须携带
    // 该 text part 在消息内的真实下标，否则按 0 查到 thinking 块渲染成空气泡。
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "assistant",
        timestamp: "2026-08-19T16:01:00Z",
        model: "anthropic.deepseek-v4-flash",
        content_parts: [
          { type: "thinking", thinking: "let me think" },
          { type: "text", text: "the answer" },
          { type: "tool_use", summary: "ls", tool_name: "bash", input: "ls" },
          { type: "text", text: "after tool" },
        ],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    const segments = turns.value[0].assistantTurn?.segments || [];
    const textSegs = segments.filter((s) => s.type === "text");
    expect(textSegs.length).toBe(2);
    if (textSegs[0].type === "text") expect(textSegs[0].partIndex).toBe(1);
    if (textSegs[1].type === "text") expect(textSegs[1].partIndex).toBe(3);
  });

  it("keeps every merged text part so rendering cannot drop the tail", () => {
    // 同一条消息内**连续**的多段文本会被合并进同一个气泡；段上必须留下各 part 的真实下标，
    // 否则渲染按首段 partIndex 只回查一次，后续正文会静默丢失，而复制/Fork 走合并后的
    // text 却带着全部内容 —— 两者不一致。
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "assistant",
        timestamp: "2026-08-19T16:02:00Z",
        model: "anthropic.deepseek-v4-flash",
        content_parts: [
          { type: "text", text: "part A\n\n" },
          { type: "text", text: "part B\n\n" },
          { type: "tool_use", summary: "ls", tool_name: "bash", input: "ls" },
          { type: "text", text: "part C\n" },
        ],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    const segments = turns.value[0].assistantTurn?.segments || [];
    const textSegs = segments.filter((s) => s.type === "text");
    expect(textSegs.length).toBe(2);

    if (textSegs[0].type === "text") {
      expect(textSegs[0].text).toBe("part A\n\npart B\n\n");
      expect(textSegs[0].partIndex).toBe(0);
      expect(textSegs[0].textParts).toEqual([
        { index: 0, text: "part A\n\n" },
        { index: 1, text: "part B\n\n" },
      ]);
    }
    if (textSegs[1].type === "text") {
      expect(textSegs[1].partIndex).toBe(3);
      expect(textSegs[1].textParts).toEqual([{ index: 3, text: "part C\n" }]);
    }
  });

  it("extracts WorkBuddy show_widget into WidgetSegment and ignores visualizer ACK result", () => {
    const rawMessages = ref<ChatMessage[]>([
      {
        role: "user",
        timestamp: "2026-08-29T14:40:00Z",
        model: null,
        content_parts: [{ type: "text", text: "讲讲实现" }],
      },
      {
        role: "assistant",
        timestamp: "2026-08-29T14:40:01Z",
        model: "workbuddy",
        content_parts: [{ type: "text", text: "下图把分叉结构画出来：" }],
      },
      {
        role: "assistant",
        timestamp: "2026-08-29T14:40:02Z",
        model: null,
        content_parts: [
          {
            type: "tool_use",
            tool_name: "show_widget",
            summary: "[图表: 消息树 fork 分叉示意]",
            input: JSON.stringify({
              title: "消息树 fork 分叉示意",
              widget_code: '<svg viewBox="0 0 680 380"><g class="node c-blue"><rect/></g></svg>',
            }),
            tool_use_id: "call-123",
          },
        ],
      },
      {
        role: "assistant",
        timestamp: "2026-08-29T14:40:03Z",
        model: null,
        content_parts: [
          {
            type: "tool_result",
            summary: "[show_widget result]",
            content: JSON.stringify({
              type: "visualizer_show_widget_result",
              success: true,
              title: "消息树_fork_分叉示意",
              widget_code: '<svg viewBox="0 0 680 380"><g class="node c-blue"><rect/></g></svg>',
            }),
            is_error: false,
          },
        ],
      },
      {
        role: "assistant",
        timestamp: "2026-08-29T14:40:04Z",
        model: "workbuddy",
        content_parts: [{ type: "text", text: "图有了，下面拆开讲。" }],
      },
    ]);

    const { turns } = useMessageTurns(rawMessages);
    // user turn + assistant text turn + widget turn + assistant text turn = 4 turns (no empty ACK turn!)
    expect(turns.value.length).toBe(4);
    expect(turns.value[0].userMessage).toBeDefined();

    // Turn 1: text
    expect(turns.value[1].assistantTurn?.segments[0].type).toBe("text");

    // Turn 2: widget
    const widgetTurn = turns.value[2];
    expect(widgetTurn.assistantTurn?.segments.length).toBe(1);
    const widgetSeg = widgetTurn.assistantTurn?.segments[0];
    expect(widgetSeg?.type).toBe("widget");
    if (widgetSeg?.type === "widget") {
      expect(widgetSeg.title).toBe("消息树 fork 分叉示意");
      expect(widgetSeg.code).toContain("<svg");
    }

    // Turn 3: text
    expect(turns.value[3].assistantTurn?.segments[0].type).toBe("text");
  });
});