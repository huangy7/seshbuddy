// 快捷用语的日期变量展开：{{今天}} 等占位符在填入输入框时展开为本地日期（yyyy-mm-dd）
// 日期计算复用 week-range.ts 的本地时区口径（避免 UTC 偏移把凌晨算成昨天）
// 变量名随界面语言（中文界面写 {{今天}}，英文界面写 {{today}}），故名字取自语言包；
// 展开时认**各语言**的写法，理由见 isVariableName

import { formatLocalDate, currentWeekRange } from "./week-range";
import { t } from "../../i18n";
import { SUPPORTED_LOCALES } from "../../i18n/types";

/**
 * 支持的变量名（语言包里的键）。清单与取值分列两处，是因为提示语要按当前语言列出
 * 全部变量，而展开要认**各语言**的名字——两者必须是同一份清单，否则提示语会
 * 宣传一个展不开的名字。取值表按此清单定类型，漏一项即编译不过。
 */
export const PHRASE_VARIABLE_KEYS = [
  "today",
  "yesterday",
  "thisMonday",
  "thisFriday",
  "lastMonday",
  "lastFriday",
] as const;

type PhraseVariableKey = (typeof PHRASE_VARIABLE_KEYS)[number];

function addDays(base: Date, days: number): Date {
  const d = new Date(base);
  d.setDate(d.getDate() + days);
  return d;
}

/**
 * 这个名字是不是该变量在**任一**支持语言下的写法。
 *
 * 只认当前语言的名字是不够的：快捷用语的正文是存在后端的用户数据，`{{今天}}` 是用户在中文
 * 界面下敲进去的；切到英文后，只认 `{{today}}` 的匹配会让那条短语静默失效，而用户从界面上
 * 看不出原因，也没有任何提示告诉他要改成英文写法。
 *
 * 各语言的写法必须现取自语言包，不能写成字面量：代码里的名字与包里的取值一旦分家，改包的人
 * 不会知道这里还有一份副本，两边静默漂移。**闸门在这件事上帮不上忙**——规则 1 只认 CJK，
 * zh/ja 的写法（`今天`、`今日`）写成字面量会被拦下，en/de 的（`this Monday`、`heute`）是纯
 * ASCII，闸门一声不响（实测 `const __de = "heute";` → exit 0）。故这条靠的是「名字只有一处
 * 来源」，不是靠闸门兜底。
 *
 * 必须用三参形式（第二参传 undefined）：第二参是具名插值，把 `{ locale }` 传在第二参会被
 * **静默忽略**，于是每个 locale 都取回当前语言的名字，别名看似生效、实则只在当前语言下匹配。
 */
function isVariableName(key: PhraseVariableKey, name: string): boolean {
  return SUPPORTED_LOCALES.some(
    (locale) => name === t(`assistant.phraseVariables.${key}`, undefined, { locale })
  );
}

/** 变量表：key 为 {{}} 内的名字（任一支持语言下的写法）。新增变量只需加一行键 + 一行取值 + 一条测试 */
function resolveVariable(name: string, now: Date): string | null {
  const week = currentWeekRange(now);
  const monday = new Date(week.start + "T00:00:00");
  const values: Record<PhraseVariableKey, string> = {
    today: formatLocalDate(now),
    yesterday: formatLocalDate(addDays(now, -1)),
    thisMonday: week.start,
    thisFriday: week.end,
    lastMonday: formatLocalDate(addDays(monday, -7)),
    lastFriday: formatLocalDate(addDays(monday, -3)),
  };
  for (const key of PHRASE_VARIABLE_KEYS) {
    if (isVariableName(key, name)) return values[key];
  }
  return null;
}

/** 展开文本中的所有 {{变量}}；未知变量原样保留（用户可见、可手改） */
export function expandPhraseVariables(text: string, now: Date = new Date()): string {
  return text.replace(/\{\{([^{}]+)\}\}/g, (raw, name: string) => {
    return resolveVariable(name.trim(), now) ?? raw;
  });
}
