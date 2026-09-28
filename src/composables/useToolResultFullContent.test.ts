import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import type { Locale } from "../i18n/types";
import {
  copyableToolResultContent,
  displayToolResultContent,
  toolResultTruncationSuffix,
} from "./useToolResultFullContent";

const BODY = "tool output body";

/** 四语的截断提示 —— zh 逐字等于改造前后端拼在正文里的那句原文（不含排版前缀）。 */
const NOTICE: Record<Locale, string> = {
  en: "[Content too long, truncated by the system to avoid UI freeze]",
  zh: "[内容超长，已被系统截断以避免界面卡死]",
  ja: "[内容が長すぎるため、UI のフリーズを避けるためシステムにより切り詰められました]",
  de: "[Inhalt zu lang, vom System gekürzt, um ein Einfrieren der Oberfläche zu vermeiden]",
};

/** 渲染结果是文案而不是 key —— 漏配 key 时 vue-i18n 原样返回 key，据此可判定。 */
function expectResolved(text: string) {
  expect(text).not.toContain("common.toolResult.truncatedSuffix");
}

afterEach(() => {
  setLocale("en");
});

describe("tool_result 截断提示（结构化标志 → 前端渲染）", () => {
  it("truncated 为真时追加本地化提示，`...\\n\\n` 由前端拼", () => {
    for (const locale of Object.keys(NOTICE) as Locale[]) {
      setLocale(locale);
      const rendered = displayToolResultContent("k1", BODY, true);
      expectResolved(rendered);
      expect(rendered).toBe(`${BODY}...\n\n${NOTICE[locale]}`);
    }
  });

  it("zh 与改造前后端拼进正文的那句逐字相同", () => {
    setLocale("zh");
    expect(displayToolResultContent("k2", BODY, true)).toBe(
      `${BODY}...\n\n[内容超长，已被系统截断以避免界面卡死]`
    );
  });

  it("未截断（false / undefined）时正文原样返回，不带任何提示", () => {
    for (const locale of Object.keys(NOTICE) as Locale[]) {
      setLocale(locale);
      expect(displayToolResultContent("k3", BODY, false)).toBe(BODY);
      expect(displayToolResultContent("k3", BODY)).toBe(BODY);
      expect(toolResultTruncationSuffix(false)).toBe("");
      expect(toolResultTruncationSuffix(undefined)).toBe("");
    }
  });

  it("标志是唯一判据：同一份正文只随 truncated 变化", () => {
    setLocale("en");
    const off = displayToolResultContent("k4", BODY, false);
    const on = displayToolResultContent("k4", BODY, true);
    expect(on).not.toBe(off);
    expect(on.startsWith(off)).toBe(true);
    expect(on.slice(off.length)).toBe(toolResultTruncationSuffix(true));
  });

  it("复制文本与渲染文本一致：截断时同样带提示", async () => {
    setLocale("de");
    // 非懒加载 part：`ensureToolResultFullContent` 直接返回 null，不会触达 IPC
    const copied = await copyableToolResultContent("k5", { truncated: true }, BODY);
    expect(copied).toBe(`${BODY}...\n\n${NOTICE.de}`);
    const plain = await copyableToolResultContent("k5", {}, BODY);
    expect(plain).toBe(BODY);
  });

  it("切换语言后同一份正文渲染出新语言的提示（提示不烘进缓存）", () => {
    setLocale("en");
    const en = displayToolResultContent("k6", BODY, true);
    setLocale("ja");
    const ja = displayToolResultContent("k6", BODY, true);
    expect(en).not.toBe(ja);
    expect(ja.endsWith(NOTICE.ja)).toBe(true);
  });
});
