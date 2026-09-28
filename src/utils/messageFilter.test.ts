import { describe, it, expect } from "vitest";
import { isMessageVisible, type MessageVisibilityFilters } from "./messageFilter";
import type { ChatMessage } from "../types/session";

const baseFilters: MessageVisibilityFilters = { user: true, assistant: true, tool: true, thinking: true };

function msg(role: string, parts: ChatMessage["content_parts"], is_meta = false): ChatMessage {
  return { role, timestamp: "", model: null, content_parts: parts, is_meta };
}

describe("isMessageVisible", () => {
  it("keeps image-bearing meta rows visible (isMeta also marks pasted image payloads)", () => {
    const m = msg("user", [{ type: "image", media_type: "image/png", data: "..." }], true);
    expect(isMessageVisible(m, baseFilters)).toBe(true);
  });

  it("treats user tool_result-only message as tool content", () => {
    const m = msg("user", [
      { type: "tool_result", summary: "", content: "output", is_error: false },
    ]);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: false, thinking: true })).toBe(false);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: true, thinking: true })).toBe(true);
  });

  it("hides user text message when user filter off", () => {
    const m = msg("user", [{ type: "text", text: "hello" }]);
    expect(isMessageVisible(m, { user: false, assistant: true, tool: true, thinking: true })).toBe(false);
  });

  it("hides user message when both user and tool filters off (mixed text + tool)", () => {
    const m = msg("user", [
      { type: "text", text: "run this" },
      { type: "tool_use", summary: "cmd", tool_name: "Bash", input: "cmd" },
    ]);
    expect(isMessageVisible(m, { user: false, assistant: true, tool: false, thinking: true })).toBe(false);
  });

  it("hides assistant tool-only message when tool filter off", () => {
    const m = msg("assistant", [
      { type: "tool_use", summary: "cmd", tool_name: "Bash", input: "cmd" },
      { type: "tool_result", summary: "", content: "out", is_error: false },
    ]);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: false, thinking: true })).toBe(false);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: true, thinking: true })).toBe(true);
  });

  it("keeps assistant text message when tool filter off but assistant filter on", () => {
    const m = msg("assistant", [
      { type: "tool_use", summary: "cmd", tool_name: "Bash", input: "cmd" },
      { type: "text", text: "answer" },
    ]);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: false, thinking: true })).toBe(true);
  });

  it("shows tool-only assistant message when tool on but assistant off", () => {
    const m = msg("assistant", [
      { type: "tool_use", summary: "cmd", tool_name: "Bash", input: "cmd" },
    ]);
    expect(isMessageVisible(m, { user: true, assistant: false, tool: true, thinking: true })).toBe(true);
  });

  it("hides assistant text message when assistant off", () => {
    const m = msg("assistant", [{ type: "text", text: "answer" }]);
    expect(isMessageVisible(m, { user: true, assistant: false, tool: true, thinking: true })).toBe(false);
  });

  it("user filter off still shows tool_result-only user message when tool on", () => {
    const m = msg("user", [
      { type: "tool_result", summary: "", content: "out", is_error: false },
    ]);
    expect(isMessageVisible(m, { user: false, assistant: true, tool: true, thinking: true })).toBe(true);
  });

  it("hides assistant thinking-only message when thinking filter off", () => {
    const m = msg("assistant", [{ type: "thinking", thinking: "let me think" }]);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: true, thinking: false })).toBe(false);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: true, thinking: true })).toBe(true);
  });

  it("keeps assistant thinking+text message when thinking filter off (text survives)", () => {
    const m = msg("assistant", [
      { type: "thinking", thinking: "let me think" },
      { type: "text", text: "answer" },
    ]);
    expect(isMessageVisible(m, { user: true, assistant: true, tool: true, thinking: false })).toBe(true);
  });

  it("shows thinking-only assistant message when thinking on but assistant off", () => {
    const m = msg("assistant", [{ type: "thinking", thinking: "let me think" }]);
    expect(isMessageVisible(m, { user: true, assistant: false, tool: true, thinking: true })).toBe(true);
  });

  it("hides assistant thinking-only message when both thinking off and assistant off", () => {
    const m = msg("assistant", [{ type: "thinking", thinking: "let me think" }]);
    expect(isMessageVisible(m, { user: true, assistant: false, tool: true, thinking: false })).toBe(false);
  });

  it("keeps assistant message with show_widget visible even when tool filter is off", () => {
    const m = msg("assistant", [
      {
        type: "tool_use",
        summary: "[show_widget: 消息树 fork 分叉示意]",
        tool_name: "show_widget",
        input: '{"title":"消息树","widget_code":"<svg>...</svg>"}',
      },
    ]);
    // With tool: false, it should remain visible directly as assistant visual content!
    expect(isMessageVisible(m, { user: true, assistant: true, tool: false, thinking: false })).toBe(true);
    // When assistant filter is off, assistant content is hidden
    expect(isMessageVisible(m, { user: true, assistant: false, tool: false, thinking: true })).toBe(false);
  });

  it("hides visualizer ACK tool_result when tool filter is off, shows when on", () => {
    const ack = msg("assistant", [
      {
        type: "tool_result",
        summary: "[show_widget result]",
        content: '{"type":"visualizer_show_widget_result","success":true}',
        is_error: false,
      },
    ]);
    expect(isMessageVisible(ack, { user: true, assistant: true, tool: false, thinking: true })).toBe(false);
    expect(isMessageVisible(ack, { user: true, assistant: true, tool: true, thinking: true })).toBe(true);
  });
});

