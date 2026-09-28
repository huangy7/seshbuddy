import { describe, expect, it, afterEach } from "vitest";
import { setLocale } from "../i18n";
import enCommon from "../locales/en/common.json";
import jaCommon from "../locales/ja/common.json";
import deCommon from "../locales/de/common.json";
import { formatMessagesToReadableText, parseRequestMessages } from "./reqMessages";

// 本文件只钉两件在迁移里容易被静默改坏的事：六条标签**拼出来的形状**（尤其 ` [Error]` 的
// 前导空格与 `unknown` 回退词），以及它们**各自**跟着 locale 走。取值本身不在这里抄一遍——
// 那是语言包的第二个事实来源，改了包还要改测试。所以 ja/de 断言的是「这一段 en 片段不再出现」，
// 与 format.test.ts 里 `expect(en).not.toBe(de)` 同一思路，但**逐条钉**而不是靠整体比较：
// 整体比较会被任意一条的差异满足，抓不到其余五条（本仓反复付代价的「测试没测它声称测的东西」）。
const reqBody = JSON.stringify({
  system: "You are a helper.",
  messages: [
    {
      role: "assistant",
      content: [
        { type: "image", source: { type: "base64", data: "..." } },
        { type: "tool_use", id: "t1", name: "Read", input: { path: "a.ts" } },
        { type: "tool_use", id: "t2", input: { path: "b.ts" } },
        { type: "tool_result", tool_use_id: "t1", content: "ok" },
        { type: "tool_result", tool_use_id: "t2", content: "boom", is_error: true },
      ],
    },
  ],
});

/**
 * 六条标签各自的「en 片段」——**必须是迁移后仍会出现在正文里的子串**。
 *
 * `> [Tool Use] {name}:` 与 `> [Tool Result{tag}]:` 的占位符会被替换，原样取值永远不可能
 * 出现在输出里，钉它等于恒真断言。所以钉的是去掉占位符后那截静态骨架。
 *
 * `unknownToolName` 例外，钉的是裸词 `unknown`：它与 `toolUseLabel` 是**两个独立的键**，
 * 只把回退词退回英文时输出是 `> [ツール呼び出し] unknown:`——拼好的整行反而匹配不上。
 */
const LABEL_FRAGMENTS = [
  { key: "imageContentLabel", en: "[Image Content]" },
  { key: "systemPromptHeading", en: "=== SYSTEM PROMPT ===" },
  { key: "toolUseLabel", en: "[Tool Use]" },
  { key: "unknownToolName", en: "unknown" },
  { key: "errorTag", en: " [Error]" },
  { key: "toolResultLabel", en: "[Tool Result" },
] as const;

/**
 * ja/de 的取值**就是** en 词形的键。
 *
 * `systemPromptHeading` 在 T6 的术语统一项里被收敛成英文全大写（纯文本导出里的机器可读标题，
 * 四语一律保留英文词形，de 用连字符形态），所以「ja 下不再停在英文」这条**对它不再适用**——
 * 它现在就是英文，那正是裁决的结果。下面两条守卫（取值非空、渲染确实用了包值）对它照旧成立，
 * 而它们才是防「标签被静默清空」的那两条。
 */
const EN_FORM_IS_CANONICAL = new Set<string>(["systemPromptHeading"]);

/** 含占位符的包值按 `{...}` 切开，只留长度 > 1 的静态骨架（与上面逐条钉用的是同一口径）。 */
function staticSegments(value: string): string[] {
  return value.split(/\{[^}]*\}/).filter((s) => s.trim().length > 1);
}

/**
 * 触发截断（>32KB）的报文：截断后缀是**写进解析结果**的文案，是本用例要钉的那一处。
 */
const truncatedBody = JSON.stringify({
  messages: [
    {
      role: "user",
      content: [{ type: "tool_result", tool_use_id: "t1", content: "x".repeat(40000) }],
    },
  ],
});

/** 非空断言要读的那两份包（en/zh 不读：逐字规则钉住了它们，另有专门用例）。 */
const PACKS = { ja: jaCommon, de: deCommon };

describe("formatMessagesToReadableText 的标签", () => {
  afterEach(() => setLocale("en"));

  it("en 下六条标签逐字保持迁移前的形状", () => {
    setLocale("en");
    const text = formatMessagesToReadableText(reqBody);
    expect(text).toContain("=== SYSTEM PROMPT ===");
    expect(text).toContain("[Image Content]");
    expect(text).toContain("> [Tool Use] Read:");
    expect(text).toContain("> [Tool Use] unknown:");
    expect(text).toContain("> [Tool Result]:");
    // 前导空格是排版的一部分：错误标签拼在 `> [Tool Result` 之后，不是独立一行
    expect(text).toContain("> [Tool Result [Error]]:");
  });

  for (const { key, en } of LABEL_FRAGMENTS) {
    for (const locale of ["ja", "de"] as const) {
      it(`${key} 在 ${locale} 下不再停在英文，且渲染确实取到了非空的包值`, () => {
        setLocale(locale);
        const text = formatMessagesToReadableText(reqBody);
        // 逐条钉：这一条退回英文时，只有这一条会红
        // （`EN_FORM_IS_CANONICAL` 里的键例外：它的包值本来就是 en 词形，见上表注释）
        if (!EN_FORM_IS_CANONICAL.has(key)) expect(text).not.toContain(en);
        // ★ 非空断言（评审实测补）：只有上面那条时，**把包值清成 `""` 本用例仍恒真**——
        // 空串不含任何 en 片段，而 `check-i18n.mjs` 没有「空值」规则，这条方向没有任何守卫。
        // 后果不是报错，是渲染出的 diff 正文**静默少掉** ` [Error]` 这类标签。
        // 故这里读**语言包自己**的值：不是把取值抄进测试（那会成为第二个事实来源、
        // 改了包还要改测试），而是断言「它非空，且渲染确实用了它」。
        const packValue = (PACKS[locale].reqMessages as Record<string, string>)[key];
        expect(packValue.trim().length, `${locale}.reqMessages.${key} 取值为空`).toBeGreaterThan(0);
        // 含占位符的键（`> [Tool Use] {name}:`）整串不会出现在输出里——占位符被替换掉了，
        // 钉整串等于恒真。按占位符切开逐段钉静态骨架，与文件头「钉骨架不钉整串」同一条理由。
        for (const seg of packValue.split(/\{[^}]*\}/).filter((s) => s.length > 1)) {
          expect(text, `${locale}.reqMessages.${key} 的静态段 ${JSON.stringify(seg)} 未出现在正文里`).toContain(
            seg,
          );
        }
      });
    }
  }

  it("ja/de 的整篇正文都不再等于 en", () => {
    setLocale("en");
    const en = formatMessagesToReadableText(reqBody);
    setLocale("ja");
    const ja = formatMessagesToReadableText(reqBody);
    setLocale("de");
    const de = formatMessagesToReadableText(reqBody);
    expect(ja).not.toBe(en);
    expect(de).not.toBe(en);
  });

  it("zh 逐字保持原文英文——本批六条的原文就是英文", () => {
    setLocale("zh");
    const text = formatMessagesToReadableText(reqBody);
    expect(text).toContain("=== SYSTEM PROMPT ===");
    expect(text).toContain("[Image Content]");
    expect(text).toContain("> [Tool Use] Read:");
    expect(text).toContain("> [Tool Result [Error]]:");
  });

  it("image 块的 text 在解析期求值，切语言后重新解析即跟随新语言", () => {
    // 解析结果按「reqBody + locale」缓存，故同一条报文换语言后必须重新解析，
    // 否则块里的 text 会停在解析那一刻的语言。ApiLogMessagesView 的兜底分支渲染的正是它。
    // 同型的还有写进 `toolOutput` 的 `truncatedSuffix`（见下一条用例）。
    setLocale("en");
    const body = JSON.stringify({ messages: [{ role: "user", content: [{ type: "image" }] }] });
    expect(parseRequestMessages(body).messages[0].contentBlocks[0].text).toBe("[Image Content]");
    setLocale("ja");
    expect(parseRequestMessages(body).messages[0].contentBlocks[0].text).toBe(
      jaCommon.reqMessages.imageContentLabel,
    );
  });

  it("同一 reqBody 换语言后重新解析：整份输出是同一种语言，不再新旧混排", () => {
    // 截断后缀是**写进解析结果**的文案（`contentBlocks[].toolOutput`），与同一次输出里
    // 现取 t() 的 `> [Tool Result]` 标签来源不同。缓存键不含 locale 时，命中缓存的第二次
    // 解析会把旧语言的截断后缀与新语言的标签拼进同一份正文——这正是本条要钉的症状。
    setLocale("en");
    const en = formatMessagesToReadableText(truncatedBody);
    setLocale("ja");
    const ja = formatMessagesToReadableText(truncatedBody);

    // 两段静态骨架分别只属于 en / ja 的包值（占位符 {count} 处会被数字替换，不能整串钉）
    const [enSegment] = staticSegments(enCommon.reqMessages.truncatedSuffix);
    const [jaSegment] = staticSegments(jaCommon.reqMessages.truncatedSuffix);
    expect(enSegment).not.toBe(jaSegment);

    expect(ja).not.toContain(enSegment);
    expect(ja).toContain(jaSegment);
    // 反向：en 下也不能混进 ja 的截断后缀（防止「两边都停在 ja」被误判为一致）
    expect(en).toContain(enSegment);
    expect(en).not.toContain(jaSegment);
    expect(ja).not.toBe(en);
  });
});
