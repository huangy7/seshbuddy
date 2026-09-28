import { describe, expect, it } from "vitest";
import { cleanUserText, formatDialogSessionTitle } from "./textClean";

describe("cleanUserText", () => {
  it("cleans skill command XML wrappers and image placeholders", () => {
    const raw = `<command-message>superpowers:systematic-debugging</command-message>
<command-name>/superpowers:systematic-debugging</command-name>
<command-args>几个问题 1、[Image #1] 我没有勾选工具调用 2、[Image #2] 测试 3、[Image #3] 会话内搜索</command-args>`;

    const cleaned = cleanUserText(raw);
    expect(cleaned).toBe("几个问题 1、我没有勾选工具调用 2、测试 3、会话内搜索");
  });

  it("handles normal prompt text without tags", () => {
    expect(cleanUserText("Hello world")).toBe("Hello world");
  });

  it("strips system-reminder with attributes (WorkBuddy user-context)", () => {
    const raw = `<system-reminder data-role="user-context">项目上下文</system-reminder>真实问题`;
    expect(cleanUserText(raw)).toBe("真实问题");
  });

  it("takes the last user_query block (WorkBuddy)", () => {
    const raw = `<user_query>旧问题</user_query>中间内容<user_query>新问题</user_query>`;
    expect(cleanUserText(raw)).toBe("新问题");
  });

  it("drops cb_summary compaction lines", () => {
    expect(cleanUserText("<cb_summary>压缩摘要</cb_summary>")).toBe("");
  });
});

describe("formatDialogSessionTitle", () => {
  it("wraps short title with quotation marks", () => {
    expect(formatDialogSessionTitle("登录问题排查")).toBe("“登录问题排查”");
  });

  it("collapses multi-line prompts and extra spaces into a single line", () => {
    const multiLine = `第一行问题描述
    第二行具体说明
    第三行复现步骤`;
    expect(formatDialogSessionTitle(multiLine)).toBe("“第一行问题描述 第二行具体说明 第三行复现步骤”");
  });

  it("truncates very long prompts with ellipsis", () => {
    const longPrompt = "A".repeat(100);
    const formatted = formatDialogSessionTitle(longPrompt, 50);
    expect(formatted).toBe(`“${"A".repeat(50)}…”`);
  });

  it("handles empty or whitespace strings gracefully", () => {
    expect(formatDialogSessionTitle("")).toBe("“Untitled session”");
    expect(formatDialogSessionTitle("   \n\t  ")).toBe("“Untitled session”");
  });
});
