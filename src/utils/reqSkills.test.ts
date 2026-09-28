import { describe, it, expect } from "vitest";
import { parseSkillInfo, hasSkillInfo } from "./reqSkills";

// 模拟真实结构：JSON 字符串值内部的换行是字面 \n
const REQ_BODY = JSON.stringify({
  model: "claude-x",
  messages: [
    {
      role: "user",
      content: [
        {
          type: "text",
          text: "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n- caveman: Ultra-compressed communication mode. Cuts token usage ~75%.\n- design-an-interface: Generate multiple radically different interface designs.\n- plugin:review: Review code changes.\n\nWhen the task at hand matches, invoke it.\n</system-reminder>",
        },
      ],
    },
    {
      role: "assistant",
      content: [
        { type: "tool_use", name: "Skill", input: { skill: "caveman" } },
        { type: "tool_use", name: "Bash", input: { command: "ls" } },
      ],
    },
    {
      role: "assistant",
      content: [{ type: "tool_use", name: "Skill", input: { skill: "caveman" } }],
    },
    {
      role: "assistant",
      content: [{ type: "tool_use", name: "Skill", input: { skill: "plugin:review" } }],
    },
  ],
});

describe("parseSkillInfo", () => {
  it("解析可用 skills 清单（名称+描述）", () => {
    const info = parseSkillInfo(REQ_BODY);
    expect(info.available).toHaveLength(3);
    expect(info.available[0]).toMatchObject({ name: "caveman" });
    expect(info.available[0].description).toContain("Ultra-compressed");
    expect(info.available[2].name).toBe("plugin:review");
  });

  it("统计 Skill 工具调用（名称+次数，按次数降序）", () => {
    const info = parseSkillInfo(REQ_BODY);
    expect(info.invoked).toEqual([
      { name: "caveman", count: 2 },
      { name: "plugin:review", count: 1 },
    ]);
  });

  it("空/坏输入不抛出", () => {
    expect(parseSkillInfo(null)).toEqual({ available: [], invoked: [] });
    expect(parseSkillInfo("not json")).toEqual({ available: [], invoked: [] });
    expect(parseSkillInfo('{"messages": []}')).toEqual({ available: [], invoked: [] });
  });

  it("hasSkillInfo 快速判断", () => {
    expect(hasSkillInfo(REQ_BODY)).toBe(true);
    expect(hasSkillInfo('{"messages":[]}')).toBe(false);
    expect(hasSkillInfo(null)).toBe(false);
  });
});
