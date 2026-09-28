import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "../i18n";
import {
  CLI_DEFINITIONS,
  cliInstallHint,
  cliPermissionHint,
  cliPermissionLabel,
  type CliDefinition,
  type CliId,
} from "./cli";

const ALL_CLIS: CliId[] = ["claude", "codex", "gemini", "workbuddy", "dsh", "antigravity"];

/** 解析结果必须是文案：漏配 key 时 vue-i18n 原样返回 key，据此可判定。 */
function expectResolved(text: string) {
  expect(text.startsWith("cli.")).toBe(false);
  expect(text.length).toBeGreaterThan(0);
}

afterEach(() => {
  setLocale("en");
});

describe("CLI 文案解析器", () => {
  it("六条 CLI 的安装提示都解析成文案", () => {
    for (const id of ALL_CLIS) {
      expectResolved(cliInstallHint(CLI_DEFINITIONS[id]));
    }
  });

  it("无平台分支的条目（WorkBuddy / DSH）不落空", () => {
    expect(cliInstallHint(CLI_DEFINITIONS.workbuddy)).toContain("WorkBuddy");
    expect(cliInstallHint(CLI_DEFINITIONS.dsh)).toBe("Install DSH");
  });

  it("权限文案按 CLI 区分，无权限语义的条目为空", () => {
    expect(cliPermissionLabel(CLI_DEFINITIONS.claude)).toBe("Skip permission checks");
    expect(cliPermissionLabel(CLI_DEFINITIONS.codex)).toBe("Bypass approvals and sandbox");
    // 空文案是「该 CLI 不参与权限开关」的语义，集成设置据此过滤，不能退化成 key
    expect(cliPermissionLabel(CLI_DEFINITIONS.workbuddy)).toBe("");
    expect(cliPermissionLabel(CLI_DEFINITIONS.dsh)).toBe("");
    for (const id of ALL_CLIS) {
      expectResolved(cliPermissionHint(CLI_DEFINITIONS[id]));
    }
  });

  it("安装命令里的 @ 不被当成链接消息语法", () => {
    // vue-i18n 把 `@` 当链接消息前缀，语言包里写成 {'@'} 字面量插值才能原样输出
    expect(cliInstallHint(CLI_DEFINITIONS.claude)).toContain(
      "npm install -g @anthropic-ai/claude-code",
    );
  });

  it("切换语言后解析结果随之改变", () => {
    const claude = CLI_DEFINITIONS.claude;
    expect(cliInstallHint(claude)).toContain("Claude Code CLI not found");
    expect(cliPermissionLabel(claude)).toBe("Skip permission checks");

    setLocale("zh");

    expect(cliInstallHint(claude)).toContain("未检测到 Claude Code CLI");
    expect(cliInstallHint(claude)).toContain("npm install -g @anthropic-ai/claude-code");
    expect(cliPermissionLabel(claude)).toBe("跳过权限检查");
  });

  it("Windows 下取 .windows 分支", async () => {
    vi.resetModules();
    vi.stubGlobal("navigator", {
      userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
    });
    const win = await import("./cli");
    const def: CliDefinition = win.CLI_DEFINITIONS.claude;
    expect(win.cliInstallHint(def)).toContain("%APPDATA%");
    // 有平台分支与无平台分支的条目都不能落到 key 上
    expect(win.cliInstallHint(win.CLI_DEFINITIONS.workbuddy)).toContain("WorkBuddy");
    vi.unstubAllGlobals();
  });
});
