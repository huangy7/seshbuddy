import { describe, it, expect } from "vitest";
import { expandPhraseVariables } from "./phrase-variables";
import { currentLocale, setLocale, t } from "../../i18n";

// 2026-08-11 是周二；本周一 = 08-10，本周五 = 08-14，上周一 = 08-03，上周五 = 08-07
const TUESDAY = new Date(2026, 7, 11, 15, 0, 0);

// 变量名随界面语言，测试跑 en 基准：{{今天}} 在 en 下写作 {{today}}
const today = t("assistant.phraseVariables.today");
const yesterday = t("assistant.phraseVariables.yesterday");
const thisMonday = t("assistant.phraseVariables.thisMonday");
const thisFriday = t("assistant.phraseVariables.thisFriday");
const lastMonday = t("assistant.phraseVariables.lastMonday");
const lastFriday = t("assistant.phraseVariables.lastFriday");

describe("expandPhraseVariables", () => {
  it("展开全部支持的变量", () => {
    const out = expandPhraseVariables(
      `总结 {{${thisMonday}}} 至 {{${today}}} 的工作；对照 {{${lastMonday}}}~{{${lastFriday}}}；昨天={{${yesterday}}}；周五={{${thisFriday}}}`,
      TUESDAY,
    );
    expect(out).toBe(
      "总结 2026-08-10 至 2026-08-11 的工作；对照 2026-08-03~2026-08-07；昨天=2026-08-10；周五=2026-08-14",
    );
  });

  it("未知变量原样保留", () => {
    expect(expandPhraseVariables(`看看 {{next year}} 的 {{${today}}}`, TUESDAY)).toBe(
      "看看 {{next year}} 的 2026-08-11",
    );
  });

  it("无变量文本原样返回", () => {
    expect(expandPhraseVariables("没有任何变量", TUESDAY)).toBe("没有任何变量");
  });

  it("同一变量出现多次全部展开", () => {
    expect(expandPhraseVariables(`{{${today}}}/{{${today}}}`, TUESDAY)).toBe(
      "2026-08-11/2026-08-11",
    );
  });

  it("中文界面的写法在中文界面下展开", () => {
    const previous = currentLocale.value;
    try {
      setLocale("zh");
      expect(expandPhraseVariables("{{今天}}", TUESDAY)).toBe("2026-08-11");
      expect(expandPhraseVariables("{{上周五}}", TUESDAY)).toBe("2026-08-07");
    } finally {
      setLocale(previous);
    }
  });

  // 快捷用语正文是存后端的用户数据：中文界面敲的 {{今天}} 必须切到任何语言后都还能展开，
  // 否则用户切换语言就会让存好的短语静默失效。别名覆盖全部支持语言，不只中英两向。
  it("变量名跨语言：任一语言的写法在任何界面语言下都展开", () => {
    const previous = currentLocale.value;
    try {
      setLocale("en");
      expect(expandPhraseVariables("{{今天}}", TUESDAY)).toBe("2026-08-11");
      expect(expandPhraseVariables("{{昨天}}", TUESDAY)).toBe("2026-08-10");
      expect(expandPhraseVariables("{{heute}}", TUESDAY)).toBe("2026-08-11");
      setLocale("zh");
      expect(expandPhraseVariables("{{today}}", TUESDAY)).toBe("2026-08-11");
      expect(expandPhraseVariables("{{this Friday}}", TUESDAY)).toBe("2026-08-14");
    } finally {
      setLocale(previous);
    }
  });
});
