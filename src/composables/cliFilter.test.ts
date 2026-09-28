import { describe, expect, it } from "vitest";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";
import { sessionIdentityKey } from "../types/session";
import {
  resolveFeatureCliId,
  resolveVisibleCliIds,
  toggleVisibleCli,
} from "./cliFilter";

function option(id: CliId): CliOption {
  return { ...CLI_DEFINITIONS[id], hasSessions: true, hasBinary: true };
}

describe("CLI 对话过滤", () => {
  it("全选模式会纳入新发现的 CLI", () => {
    expect(resolveVisibleCliIds({ mode: "all" }, ["claude", "codex"]))
      .toEqual(["claude", "codex"]);
  });

  it("自定义选择不会自动纳入新发现的 CLI", () => {
    expect(resolveVisibleCliIds({ mode: "custom", cliIds: ["claude"] }, ["claude", "codex"]))
      .toEqual(["claude"]);
  });

  it("自定义选择按可用 CLI 的稳定顺序返回", () => {
    expect(resolveVisibleCliIds({ mode: "custom", cliIds: ["codex", "claude"] }, ["claude", "codex"]))
      .toEqual(["claude", "codex"]);
  });

  it("取消最后一个 CLI 后保留空的自定义选择", () => {
    expect(toggleVisibleCli({ mode: "custom", cliIds: ["claude"] }, ["claude"], "claude"))
      .toEqual({ mode: "custom", cliIds: [] });
  });

  it("从全选取消一个 CLI 时转为其余 CLI 的自定义选择", () => {
    expect(toggleVisibleCli({ mode: "all" }, ["claude", "codex"], "claude"))
      .toEqual({ mode: "custom", cliIds: ["codex"] });
  });

  it("切换已发现来源时保留尚无会话的已选来源", () => {
    expect(toggleVisibleCli(
      { mode: "custom", cliIds: ["codex", "antigravity"] },
      ["claude", "codex"],
      "claude",
    )).toEqual({ mode: "custom", cliIds: ["claude", "codex", "antigravity"] });
  });
});

describe("功能页 CLI 选择", () => {
  const candidates = [option("claude"), option("codex")];

  it("优先使用入口对话所属且可用的 CLI", () => {
    expect(resolveFeatureCliId(candidates, "codex", "claude")).toBe("codex");
  });

  it("入口 CLI 不可用时使用持久化选择", () => {
    expect(resolveFeatureCliId([option("codex")], "claude", "codex")).toBe("codex");
  });

  it("入口与持久化选择都不可用时使用第一个候选 CLI", () => {
    expect(resolveFeatureCliId([option("codex"), option("claude")], "gemini", "dsh"))
      .toBe("codex");
  });

  it("没有能力候选 CLI 时返回 undefined", () => {
    expect(resolveFeatureCliId([], "claude", "codex")).toBeUndefined();
  });
});

describe("对话身份", () => {
  it("相同路径的不同 CLI 产生不同且稳定的身份键", () => {
    const claude = sessionIdentityKey({ cliId: "claude", filePath: "/tmp/shared.jsonl" });
    const codex = sessionIdentityKey({ cliId: "codex", filePath: "/tmp/shared.jsonl" });

    expect(claude).toBe("claude\u0000/tmp/shared.jsonl");
    expect(codex).toBe("codex\u0000/tmp/shared.jsonl");
    expect(claude).not.toBe(codex);
  });
});
