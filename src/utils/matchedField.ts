import { t } from "../i18n";
import type { MatchedField } from "../types/session";

/**
 * 命中字段标签：后端只回结构化字段名（`MatchedField`），文案由语言包渲染。
 *
 * 两个搜索入口（GlobalSearch / DashboardView）共用本函数——此前各存一份逐字相同的副本，
 * 两份都以 `return ""` 收尾，于是后端新增变体时**两边都静默渲染空标签**：用户看到一个
 * 没有说明的裸 id，正是「把内部标识符推给用户看」那一类缺陷。
 *
 * 穷举由 `default` 里的 `never` 哨兵强制：漏写变体时 `field` 落进 `default` 而不再是
 * `never`，那一行是**编译错误**（`vue-tsc` 挡在构建期），不是运行时的空标签。
 * ⚠️ 该哨兵只管 TS 内部穷举：**Rust 侧加变体而这里的联合没跟上时它不响**
 * （联合没变，`default` 不可达），字面量会被粘上 snippet。那条漂移由
 * `matchedField.parity.test.ts` 比对 Rust 枚举与 TS 联合钉住。
 *
 * `null` 显式返回空串：它表示「后端无从分类」，没有可展示的字段标签。
 * `content` 也返回空串：正文命中不需要前缀说明，加前缀会把每条结果的正文都改写一遍。
 */
export function matchedFieldLabel(field: MatchedField | null): string {
  switch (field) {
    case "title":
      return t("session.searchResult.matchedTitle");
    case "first_message":
      return t("session.searchResult.matchedFirstMessage");
    case "session_id":
      return t("session.searchResult.matchedSessionId");
    case "content":
      return "";
    case null:
      return "";
    default: {
      const exhaustive: never = field;
      return exhaustive;
    }
  }
}
