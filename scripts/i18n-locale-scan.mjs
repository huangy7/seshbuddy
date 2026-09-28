#!/usr/bin/env node
/**
 * 语言包的【机械派生】测量：重复键、译文漂移、复数不安全、孤儿键。
 *
 * 本批的三项工作（合并 / 复数 / 漂移）在 spec 里都带着**手工清单**，而本仓已经为此付过三次代价
 * （重复键清单 14 vs 实测 91；英文盲区清单 131/21 在计划 5 之后失效；复数清单 3 → 12 → 19
 * 两次靠人扫两次不全）。**结论是 spec 自己写下的**：「去重必须以机械派生为准，
 * 不得采信文档里的任何手工清单。」本脚本把那次派生固定下来，输出是唯一事实来源。
 *
 * **它产出的是候选，不是结论。** 每条命中都要按判据逐条裁定，裁定结果写进
 * `scripts/i18n-{dedup,plural,drift}-adjudication.json`；本脚本每报出一条候选，
 * 下游任务先在那里查一次——**不在里面的才是未处置**。
 *
 * 去重裁决集是**扁平的 `键 → 条目` 映射**（闭合用例要求顶层键集合等于当时的重复组键集合），
 * 装不下注释，故这条指引写在这里：**T5 那 128 条合并的逐条「为什么这个耦合可接受」理由，
 * 随条目一起在提交 `344fb9d1` 里被删掉，原文在 `git show 545d0c56:scripts/i18n-dedup-adjudication.json`**
 * （128 条 `action: "merge"` 条目，每条含 `into` 与理由；`545d0c56..344fb9d1` 这个区间
 * 即「条目还在」到「条目被删」的那一步）。
 *
 * ## 口径（先写下来再动手——两种口径差 90 组；⚠️ 该数是 T1 基线，HEAD 见下）
 *
 * `--duplicates` 的判据是**四语取值元组相同**：两个键的 `en`/`zh`/`ja`/`de` 逐项相同。
 * **不是**「四语彼此全等」（如 `Headers` / `Raw JSON` 这类标识符）——后者实测只有 1 组，
 * 与元组口径相差两个数量级。
 *
 * ⚠️ **订正（2026-09-23，收口复测）**：上面这段的计数是 **T1 基线**。在 T1 提交（`6e08d008`）
 * 上复测为：元组口径 **93 组（非空 92 + 全空 1）**、「四语彼此全等」口径 **2 组（非空 1 + 全空 1）**，
 * 与原文「93 / 92」「只有 1 组」对得上；标题的「差 90 组」按非空口径复测为 91（92 − 1）、
 * 按总口径也是 91（93 − 2），原文的 90 是一个近似记法。**HEAD 实测**：元组口径
 * **5 组（非空 4 + 全空 1）**、
 * 「四语彼此全等」口径 **0 组非空**（唯一一组是全空值组，见下节），差 **4 组**——
 * 「相差两个数量级」在 HEAD 已不成立。`Headers` / `Raw JSON` 两个键在 HEAD 各只剩一个成员
 * （`chat.apiDetail.headers` / `api-log.requestDetail.rawJson`），**不再构成组**，
 * 故那句括号已不再指代本计数的对象。保留原值是为了让「它被什么取代」留在原处，不再作为依据。
 *
 * ## 空值组：92 与 93 的唯一来源（⚠️ T1 基线计数，HEAD 为 4 与 5）
 *
 * `cli.permissionLabel.workbuddy` 与 `cli.permissionLabel.dsh` 四语取值**全为空串**，
 * 元组相同故构成一个重复组。它在「重复组」的计数里算不算，就是 93 与 92 的差。
 * **两个数都报**：`--duplicates` 的分组列表里给它打 `[全空值]` 标记，
 * `--summary` 同时给出 `duplicateGroups`（93）与 `duplicateGroupsNonEmpty`（92）。
 *
 * ⚠️ **订正（2026-09-23，收口复测）**：93 / 92 是 **T1 基线**。**HEAD 实测**（`--summary` 输出）：
 * `duplicateGroups=5`、`duplicateGroupsNonEmpty=4`、`duplicateGroupsAllEmpty=1`——
 * T5 的 128 键合并与 T4 的 2 条死键删除各解散了一批组。全空值组仍是
 * `cli.permissionLabel.workbuddy` 与 `cli.permissionLabel.dsh` 这一组（实测），
 * 故「两个数恒差 1」这条口径本身未变，变的只是量级。保留原值是为了让「它被什么取代」留在原处。
 *
 * ## 各模式的假阳性类（逐条实测过，不要往上无限加过滤器）
 *
 * `--plural` 用**真 vue-i18n**（仓内 `node_modules` 的 `createI18n({legacy:false})`）
 * 在 `count = 1` 下逐键逐语**真渲染**，再按「计数后紧跟复数屈折词」的形态判候选。
 * 形态判据是**候选生成器**，不是语法判定器——实测 12 条候选里 2 条是假阳性
 * （均已裁定为 `keep`），故**裁定的不安全数是 10**：
 *
 *   | 键 | 渲染 | 为什么是假阳性 |
 *   |---|---|---|
 *   | `api-log.trafficPanel.errorCount` | de `1 Fehler` | `Fehler` 单复数同形 |
 *   | `session.sessionTree.loadingListCount` | de `… 1 empfangen` | 分词，不是名词 |
 *
 * ⚠️ **订正（2026-09-23，收口复测）**：12 / 2 / 10 是 **T1 基线**，且它们是**裁决集**
 * （`scripts/i18n-plural-adjudication.json`）的属性，不是 HEAD 工具输出的属性。
 * 实测该裁决集共 **15 条**（7 `pluralize` / 6 `keep` / 2 `delete`）——上表这两条形态误报
 * 正在那 6 条 `keep` 里，与原文自洽。**HEAD 实测 `pluralCandidates=6`**：七条 `pluralize`
 * 已改成真复数语法、两条死键已删，剩下的 6 条正是裁决集里 `action === "keep"` 的那 6 条。
 * 保留原值是为了让「它被什么取代」留在原处。
 *
 * **12 → 14 的来源**：后加的 2 条 `pluralize` 是导出提示
 * （`app.batch.exportOutcome.merged` / `.separate`）——`count = 1` 时渲染出
 * `Merged 1 sessions into one file.` / `1 Sitzungen …`，已改成 `|`-form。
 * 两条都**不在** `pluralCandidates` 里（改完就不该在），故上段的候选数不受影响。
 * **14 → 15 的来源**：T4 又加了一条 `keep`
 * （`api-profile.scopeAllocation.statusDirsInconsistentTooltip`——当前不可达，理由见裁决集），
 * 它是假阳性、留在候选里，故候选数 5 → 6。
 *
 * ⚠️ **本段的数字是手工维护的，必然漂移——引用前先重测，不要照抄。** 它已被改过两次：
 * T2 的修复轮把 12 → 14，T4 加那条 `keep` 又让它变成 15，两次都没有任何东西在看着它。
 * 重测：裁决集条数与各 `action` 分布直接数 `scripts/i18n-plural-adjudication.json` 的条目，
 * 候选数看 `--plural` 末行的 `pluralCandidates=`。**本注释的计数是第二个事实来源，
 * 别让它取代工具输出。**
 *
 * 形态**按语言分别判**：de 的词尾形态不能拿去套 en 的渲染，否则 `1 idle`（形容词）
 * 这类会被误报——实测把 de 形态套到 en 上会多出 1 条假阳性。`zh`/`ja` 无复数屈折，
 * 故不设形态，这两语从不单独构成候选。
 * 残留的几类靠报告里逐条裁定，比靠正则可靠。**每加一个过滤器就是加一个事实来源**，
 * 而本仓的教训正是「第二个事实来源是缺陷的住处」。
 *
 * ## `--orphans` 的豁免：少而具名，逐条打印
 *
 * 引用判据是「键名以引号形态出现在 `src/**` 的 `.ts`/`.vue` 里（排除 `*.test.ts`）」。
 * 三种情况**结构上不可能**被它命中，故列为豁免，并在输出里以 `[豁免]` 单独打印：
 *
 * 1. `assistant.phraseVariables.*` —— 动态拼键（`phrase-variables.ts` 里
 *    ``t(`assistant.phraseVariables.${key}`)``），前缀豁免。
 * 2. `cli.installHint.*` 的 `.windows` / `.unix` 后缀 —— 动态拼后缀
 *    （`src/types/cli.ts` 里 `` `${base}.windows` ``），**只豁免这两个后缀**，
 *    前缀本身不豁免（`cli.installHint.claude` 等基键是字面量，照查）。
 * 3. 由 `src-tauri/` 消费的键 —— 后端用 `native_text("<ns>", "<key>")` 直读语言包
 *    （`native_text.rs` 的 `include_str!`），**从 Rust 源码里机械提取**，逐键列出。
 *    `tray.*` 与 `native.updateNotification*` 属此类。
 *
 * `src/changelog.ts` 的 `changeKeys` 数据表**不需要豁免**：它以**字面量**存 key，
 * 引用判据已能命中（实测该前缀下 7 键全部被引用，无孤儿）。输出里以 `[记录]` 打印这一行。
 *
 * 用法：
 *   node scripts/i18n-locale-scan.mjs --duplicates
 *   node scripts/i18n-locale-scan.mjs --drift
 *   node scripts/i18n-locale-scan.mjs --plural
 *   node scripts/i18n-locale-scan.mjs --orphans
 *   node scripts/i18n-locale-scan.mjs --summary
 *   node scripts/i18n-locale-scan.mjs --term <词形或正则>   # 术语扫描（Step 4 要求脚本产出，不凭记忆）
 *
 * 术语项的**范围**（哪些键属于这个概念）另由导出的派生函数给出（如
 * `findEllipsisFormDivergences`）——`--term` 只答「哪些键含这个词形」，答不出「这个概念的边界」，
 * 而边界一旦靠手工圈定，就会漏掉同类里其余成员。裁决集的 `keys` 必须等于派生函数的输出。
 */
import { readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { createI18n } from "vue-i18n";
import { flattenMessages } from "./check-i18n.mjs";

/** 语言集合：`en` 是基准，其余是目标语言。顺序即元组的顺序，改动它等于改口径。 */
export const LANGS = ["en", "zh", "ja", "de"];

/** `_meta.json` 是语言包元数据（`reviewed` / `translatedFrom` / `reviewer`），不是命名空间。 */
export const META_FILE = "_meta.json";

/** 命名空间清单从 `en/` 目录机械派生，**不手打**（手打会漂移）。 */
export function listNamespaces(root) {
  return readdirSync(join(root, "src/locales/en"))
    .filter((f) => f.endsWith(".json") && f !== META_FILE)
    .map((f) => f.replace(/\.json$/, ""))
    .sort();
}

/**
 * 展平一个语言包对象为 `[键, 取值]` 列表；嵌套对象用 `.` 连接。
 *
 * 直接复用闸门的 `flattenMessages`：**数组的处理口径必须与规则 3 一致**
 * （那边用 `isBranch`，数组按文本处理；自写一份 `typeof v === "object"`
 * 会把数组递归成 `0` / `1` 下标键，于是同一份包在闸门与本脚本下有两套键集）。
 */
export const flattenPack = flattenMessages;

/**
 * 读四语 × 全部命名空间，展平为 `ns.key → {en,zh,ja,de}`。
 *
 * 返回 `keys`（有序，`ns.key`）与 `values[lang][key]`。键集合不一致时**不静默**：
 * 缺键记为 `undefined`，由调用方按 `"<MISSING>"` 处理并计数——静默当成空串会把
 * 「键集合漂移」伪装成「空值组」。
 */
export function loadPacks(root, namespaces = listNamespaces(root)) {
  const values = {};
  const keys = [];
  const seen = new Set();
  for (const lang of LANGS) {
    values[lang] = {};
    for (const ns of namespaces) {
      const pack = JSON.parse(readFileSync(join(root, `src/locales/${lang}/${ns}.json`), "utf8"));
      for (const [k, v] of flattenPack(pack)) {
        const full = `${ns}.${k}`;
        values[lang][full] = v;
        if (!seen.has(full)) {
          seen.add(full);
          keys.push(full);
        }
      }
    }
  }
  return { namespaces, keys, values };
}

/** 一个键的四语取值元组（缺失记为 `<MISSING>`，与空串区分开）。 */
export function tupleOf(values, key, langs = LANGS) {
  return langs.map((l) => (values[l][key] === undefined ? "<MISSING>" : values[l][key]));
}

/** 四语取值是否全等（「四语彼此相同」口径，与元组口径不同，仅用于报告）。 */
export function isAllLangsEqual(values, key, langs = LANGS) {
  const vals = tupleOf(values, key, langs);
  return vals.every((v) => v === vals[0]);
}

/**
 * 重复组：**四语取值元组相同**的键，组内 ≥2 个键。
 *
 * 组按首次出现的键排序，组内键按字典序——**输出稳定**，否则每次跑出来的顺序不同，
 * 「和上次对不上」会被误当成组划分变了。
 */
export function findDuplicateGroups({ keys, values }) {
  const byTuple = new Map();
  for (const key of keys) {
    const t = tupleOf(values, key).join("\u0000");
    if (!byTuple.has(t)) byTuple.set(t, []);
    byTuple.get(t).push(key);
  }
  return [...byTuple.values()]
    .filter((g) => g.length > 1)
    .map((g) => {
      const sorted = [...g].sort();
      return {
        keys: sorted,
        values: Object.fromEntries(LANGS.map((l) => [l, values[l][sorted[0]]])),
        /** 四语取值全为空串——`workbuddy|dsh` 是唯一一组，92/93 之差全在这里。 */
        allEmpty: LANGS.every((l) => values[l][sorted[0]] === ""),
        allLangsEqual: isAllLangsEqual(values, sorted[0]),
      };
    })
    .sort((a, b) => a.keys[0].localeCompare(b.keys[0]));
}

/**
 * 漂移组：同一 `en` 取值、**元组不同**的组（组内 ≥2 种元组）。
 *
 * 每个元组给一个代表键与各键清单。分类是**候选分类**，供裁定用：
 *
 * - `A`：组内既有「原文是英文」的键（`zh === en`）又有「原文是中文」的键（`zh !== en`）
 *   ——`zh` 必然不同，合并会改中文界面。
 * - `B`：组内所有键的原文都是中文（`zh !== en`）而 `zh` 逐字不同——同上。
 * - `C`：`zh` 完全一致，分歧只在 `ja`/`de`——**这一类才是「同一句源文案、译文没对齐」**。
 *
 * A/B/C 与「ja/de 是否分歧」是**两条正交的轴**：A/B 里也有 ja/de 分歧的组（21 组），
 * 那是**源文案不同导致的结果**，不是译文没对齐。两个数都要报。
 */
export function findDriftGroups({ keys, values }) {
  const byEn = new Map();
  for (const key of keys) {
    const en = values.en[key];
    if (!byEn.has(en)) byEn.set(en, new Map());
    const t = tupleOf(values, key).join("\u0000");
    const m = byEn.get(en);
    if (!m.has(t)) m.set(t, []);
    m.get(t).push(key);
  }
  const out = [];
  for (const [en, m] of byEn) {
    if (m.size < 2) continue;
    const variants = [...m.values()].map((g) => {
      const sorted = [...g].sort();
      return { keys: sorted, values: Object.fromEntries(LANGS.map((l) => [l, values[l][sorted[0]]])) };
    });
    const all = variants.flatMap((v) => v.keys);
    const distinct = (lang) => new Set(all.map((k) => values[lang][k])).size;
    const enOriginal = all.filter((k) => values.zh[k] === values.en[k]);
    const zhOriginal = all.filter((k) => values.zh[k] !== values.en[k]);
    const cls = enOriginal.length && zhOriginal.length ? "A" : zhOriginal.length === all.length && distinct("zh") > 1 ? "B" : "C";
    out.push({
      en,
      variants,
      cls,
      zhDiverges: distinct("zh") > 1,
      jaDeDiverges: distinct("ja") > 1 || distinct("de") > 1,
    });
  }
  return out.sort((a, b) => a.en.localeCompare(b.en));
}

/**
 * 各语言「计数后紧跟复数屈折词」的候选形态。
 *
 * 只对**有复数屈折的语言**（en/de）判；`zh` 无屈折、`ja` 无屈折，故不设形态——
 * 这两语的渲染从不构成「语法不成立」的证据。de 的三种词尾覆盖常见的复数形态，
 * 代价是**单复数同形/分词/形容词会被误报**（见头注释的假阳性表），那是裁定的工作。
 *
 * **两个方向都要写下来，因为危险的是漏报那一边。** 上面说的是**假阳性**方向（形态命中但
 * 德语本身没问题）。本判据还有**假阴性**方向：它只看词尾，故
 * ① **`-s` 复数**（`{count} Autos` → `1 Autos`）不被 de 形态命中；
 * ② **复数与单数同形**时渲染串里根本没有可供判别的形态，判据结构上看不见。
 * 这两类都会**漏报**——而漏报的后果是「复数不安全的键在闸门与裁决集里都不存在」，
 * 比误报（多一条待裁定项）危险得多。
 *
 * 该方向已实测过（2026-09-23，收口复测）：把形态放宽到 en `(?:s|ren|feet|men|children)`、
 * de `(?:en|er|e|s)` 后，对**全部 77 个含 `{count}` 的键**逐键渲染 `count = 1`，
 * 宽口径比窄口径**多报 0 键**——即 HEAD 没有「漏报且可疑」的实例。
 * 阳性对照证明宽口径确有鉴别力：夹具 `{count} Autos`（de 渲染 `1 Autos`）与
 * `{count} children`（en 渲染 `1 children`）都是窄口径不报、宽口径报的。
 */
export const PLURAL_ADJACENCY = {
  en: /\b1\s+[A-Za-z][A-Za-z-]*s\b/,
  de: /\b1\s+[A-Za-zÄÖÜäöüß][A-Za-zÄÖÜäöüß-]*(?:en|er|e)\b/,
};

/** 字面 `(en)` 对冲写法（`vor 1 Tag(en)`）——与屈折无关，任何语言里出现即候选。 */
export const HEDGE_PATTERN = /\(en\)/;

/** 用真 vue-i18n 建实例；`messages` 形如 `{en: {ns: {...}}}`。 */
export function buildI18n(messages) {
  return createI18n({
    legacy: false,
    locale: "en",
    fallbackLocale: "en",
    messages,
    missingWarn: false,
    fallbackWarn: false,
  });
}

/**
 * 在指定语言下把某个键渲染成 `count = 1` 的字符串。
 *
 * **必须走真 vue-i18n**：正则判「有没有 `|`」判不出 `|` 在别的分支里、
 * 也判不出占位符没被替换这类问题，而 spec 的教训正是「这条约定的验收方式不能是『读一遍看看』」。
 * 已带 `|`-form 的消息由 vue-i18n 自己选分支，渲染结果即为该语言的真实输出。
 */
export function renderCount(i18n, key, lang, count = 1) {
  i18n.global.locale.value = lang;
  return i18n.global.t(key, { count });
}

/** 一条渲染结果是不是「计数后紧跟复数屈折词」的候选（含 `(en)` 对冲）。 */
export function isPluralUnsafeRender(lang, rendered) {
  if (HEDGE_PATTERN.test(rendered)) return true;
  const re = PLURAL_ADJACENCY[lang];
  return re ? re.test(rendered) : false;
}

/**
 * 复数候选：`en` 取值含 `{count}` 的键，在四语下各渲染一次 `count = 1`。
 *
 * 已带 `|`-form 的消息由 vue-i18n 选分支，**不另判**——「`|`-form 已存在的视为安全」
 * 是通过「渲染出来是什么就是什么」实现的，不是靠正则豁免。
 */
export function findPluralCandidates(messages, i18n = buildI18n(messages)) {
  const out = [];
  for (const [ns, pack] of Object.entries(messages.en)) {
    for (const [k, v] of flattenPack(pack)) {
      if (!v.includes("{count}")) continue;
      const key = `${ns}.${k}`;
      const renders = {};
      const flagged = [];
      for (const lang of LANGS) {
        const rendered = renderCount(i18n, key, lang);
        renders[lang] = rendered;
        if (isPluralUnsafeRender(lang, rendered)) flagged.push(lang);
      }
      if (flagged.length) out.push({ key, en: v, renders, flagged });
    }
  }
  return out.sort((a, b) => a.key.localeCompare(b.key));
}

/** `src/` 下的前端源文件；排除 `*.test.ts`（与闸门的过滤条件一致）。 */
export function listSourceFiles(root) {
  const out = [];
  const walk = (d) => {
    for (const n of readdirSync(d)) {
      const f = join(d, n);
      if (statSync(f).isDirectory()) walk(f);
      else if (/\.(ts|vue)$/.test(f) && !f.endsWith(".test.ts")) out.push(f);
    }
  };
  walk(join(root, "src"));
  return out.sort();
}

/** 源文件正文表：`相对路径 → 内容`。测试要能注入自己的表，否则夹具用例会因「无所属文件」空过。 */
export function collectSources(root, files = listSourceFiles(root)) {
  const out = {};
  for (const f of files) out[relative(root, f).split("\\").join("/")] = readFileSync(f, "utf8");
  return out;
}

/**
 * 键的引用点：正文里出现 `"<键>"` 或 `'<键>'` 的源文件。
 *
 * 用**引号形态**而不是裸子串：`assistant.history` 是 `assistant.history.title` 的前缀，
 * 裸子串判据会把前缀键判成「有引用」，于是真正没人用的前缀键永远不被报出。
 */
export function findReferences(key, sources) {
  return Object.keys(sources).filter((f) => sources[f].includes(`"${key}"`) || sources[f].includes(`'${key}'`));
}

/**
 * `src-tauri/` 消费的键：Rust 侧用 `native_text("<ns>", "<key>")` 直读语言包
 * （`native_text.rs` 的 `include_str!`）。**从 Rust 源码机械提取**，不手抄——
 * 手抄的清单会在下一次加托盘项时静默失效。
 */
export function findRustNativeTextKeys(root) {
  const dir = join(root, "src-tauri/src");
  const out = new Set();
  let files;
  try {
    files = readdirSync(dir, { recursive: true })
      .filter((f) => String(f).endsWith(".rs"))
      .map((f) => join(dir, String(f)));
  } catch {
    return out;
  }
  for (const f of files) {
    const src = readFileSync(f, "utf8");
    for (const m of src.matchAll(/native_text(?:_with)?\(\s*"([^"]+)"\s*,\s*"([^"]+)"/g)) {
      out.add(`${m[1]}.${m[2]}`);
    }
  }
  return out;
}

/**
 * 动态拼键的豁免规则。**少而具名**：每条给出精确的前缀/后缀与实测的出处，
 * 不用通配把整个命名空间划掉。`test` 决定某个孤儿键是否被这条规则豁免。
 */
export const DYNAMIC_KEY_EXEMPTIONS = [
  {
    id: "phraseVariables",
    reason: "动态拼键：src/components/assistant/phrase-variables.ts 里 t(`assistant.phraseVariables.${key}`)",
    test: (key) => key.startsWith("assistant.phraseVariables."),
  },
  {
    id: "cliInstallHintPlatform",
    reason: "动态拼后缀：src/types/cli.ts 里 `${base}.windows` / `${base}.unix`（只豁免这两个后缀，基键照查）",
    test: (key) => key.startsWith("cli.installHint.") && (key.endsWith(".windows") || key.endsWith(".unix")),
  },
  {
    id: "pricingProviderDisplayName",
    reason:
      "动态拼键：src/composables/usePricingCatalog.ts 里 t(`dialogs.pricing.provider.${id}`)" +
      "（providerDisplayName 是厂商显示名的唯一出口，只豁免这一层后缀，providerCurrentSession 等基键照查）",
    test: (key) => key.startsWith("dialogs.pricing.provider."),
  },
  {
    id: "backendErrorCodes",
    reason:
      "动态拼键：src/utils/invokeApp.ts 里 t(`errors.${err.code}`)——code 由后端 AppError::Coded " +
      "跨 IPC 给出，取值在运行时才确定。引用证明是闸门规则 10 的**双向闭合**：" +
      "每个 errors.* 键都必须有 coded(\"…\") 站点产出，反向亦然；" +
      "故这条前缀不是把命名空间划掉，而是由规则 10 逐键担保其被引用（前缀而非逐键清单：code 会持续新增）",
    test: (key) => key.startsWith("errors."),
  },
];

/**
 * 孤儿键：`src/`（排除 `*.test.ts`）里**无任何引用**、且不在豁免里的键。
 *
 * 返回 `orphans`（待裁定，删除是下游任务的活）与 `exempt`（逐键列出，附豁免依据）。
 * 两者**分开打印**：混在一起读的人分不清「豁免」与「干净」。
 */
export function findOrphans({ keys }, sources, { rustKeys = new Set() } = {}) {
  const orphans = [];
  const exempt = [];
  for (const key of keys) {
    if (findReferences(key, sources).length) continue;
    const rule = DYNAMIC_KEY_EXEMPTIONS.find((r) => r.test(key));
    if (rule) exempt.push({ key, reason: rule.reason });
    else if (rustKeys.has(key)) exempt.push({ key, reason: "由 src-tauri 的 native_text() 消费（Rust 直读语言包）" });
    else orphans.push(key);
  }
  return { orphans, exempt };
}

/** 跑完整测量。`--plural` 需要真 vue-i18n，故这里把 messages 也建出来。 */
export function measure(root) {
  const packs = loadPacks(root);
  const namespaces = packs.namespaces;
  const messages = {};
  for (const lang of LANGS) {
    messages[lang] = {};
    for (const ns of namespaces) messages[lang][ns] = JSON.parse(readFileSync(join(root, `src/locales/${lang}/${ns}.json`), "utf8"));
  }
  const sources = collectSources(root);
  const duplicates = findDuplicateGroups(packs);
  const drift = findDriftGroups(packs);
  const plural = findPluralCandidates(messages);
  const { orphans, exempt } = findOrphans(packs, sources, { rustKeys: findRustNativeTextKeys(root) });
  return { packs, namespaces, duplicates, drift, plural, orphans, exempt };
}

/** 术语扫描：给一个词形（正则），列出所有匹配的键与语言。 */
export function findTerm({ keys, values }, pattern) {
  const re = pattern instanceof RegExp ? pattern : new RegExp(pattern);
  const out = [];
  for (const key of keys) {
    const hits = LANGS.filter((l) => values[l][key] !== undefined && re.test(values[l][key]));
    if (hits.length) out.push({ key, hits, values: Object.fromEntries(hits.map((l) => [l, values[l][key]])) });
  }
  return out;
}

/** 一个串里的省略号形态：`…` / `...` / 两者 / 无。 */
export function ellipsisForm(s) {
  const f = [];
  if (s.includes("...")) f.push("...");
  if (s.includes("…")) f.push("…");
  return f.join("+") || "(none)";
}

/**
 * 省略号形态分歧：`ja` 或 `de` 的省略号形态与该键**自己的** `en` 形态不同的键。
 *
 * 判据是**逐键**的（Ruling 13）：`en` 是基准语言，ja/de 要与**各自那一行**的 `en` 同形，
 * 而不是全局统一成某一种——`en` 自身两种形态都在用（实测 16 键用 `...`、89 键用 `…`），
 * 全局目标会让 ja/de 与自己那一行的基准语言不一致。
 *
 * ⚠️ **订正（2026-09-23，收口复测）**：上面括号里的 16 / 89 是 **T1 基线**。
 * **HEAD 实测：`en` 含 `...` 的键 16 个、含 `…` 的键 80 个**（`...` 侧未变）。
 * 同一事实在 `scripts/i18n-drift-adjudication.json` 的省略号条目里已按 T6 复测订正为 80，
 * 本处当时漏改——同一个数在两个产物里处于两个状态，正是本仓要杀掉的「第二份事实来源」。
 * 少的 2 个来自 T5 的 128 键合并。保留原值是为了让「它被什么取代」留在原处。
 *
 * 导出它是为了让裁决集能被**判据**钉住（`--term` 只给词形、给不出范围）：
 * 术语项的 `keys` 必须等于本函数的输出，而不是某次手工圈定的子集。
 */
export function findEllipsisFormDivergences({ keys, values }) {
  return keys
    .filter(
      (k) =>
        ellipsisForm(values.ja[k]) !== ellipsisForm(values.en[k]) ||
        ellipsisForm(values.de[k]) !== ellipsisForm(values.en[k]),
    )
    .sort();
}

function main() {
  const root = fileURLToPath(new URL("..", import.meta.url));
  const argv = process.argv.slice(2);
  const r = measure(root);

  const dupNonEmpty = r.duplicates.filter((g) => !g.allEmpty).length;

  if (argv.includes("--summary")) {
    console.log(`namespaces=${r.namespaces.length}`);
    console.log(`keys=${r.packs.keys.length}`);
    console.log(`duplicateGroups=${r.duplicates.length}`);
    console.log(`duplicateGroupsNonEmpty=${dupNonEmpty}`);
    console.log(`duplicateGroupsAllEmpty=${r.duplicates.length - dupNonEmpty}`);
    console.log(`duplicateKeys=${r.duplicates.reduce((n, g) => n + g.keys.length, 0)}`);
    console.log(`driftGroups=${r.drift.length}`);
    console.log(`driftGroupsJaDeDiverging=${r.drift.filter((g) => g.jaDeDiverges).length}`);
    console.log(`driftGroupsZhDiverging=${r.drift.filter((g) => g.zhDiverges).length}`);
    console.log(`countKeys=${r.packs.keys.filter((k) => r.packs.values.en[k].includes("{count}")).length}`);
    console.log(`pluralCandidates=${r.plural.length}`);
    console.log(`orphans=${r.orphans.length}`);
    console.log(`orphanExempt=${r.exempt.length}`);
    return;
  }

  if (argv.includes("--duplicates")) {
    for (const g of r.duplicates) {
      const tag = g.allEmpty ? " [全空值]" : g.allLangsEqual ? " [四语全等]" : "";
      console.log(`\n${g.keys.length} 键${tag}  en=${JSON.stringify(g.values.en)}`);
      for (const k of g.keys) console.log(`  ${k}`);
    }
    console.log(
      `\n# duplicateGroups=${r.duplicates.length} （排除全空值组后 ${dupNonEmpty}；` +
        `全空值组 ${r.duplicates.length - dupNonEmpty}）duplicateKeys=${r.duplicates.reduce((n, g) => n + g.keys.length, 0)}`,
    );
    return;
  }

  if (argv.includes("--drift")) {
    for (const g of r.drift) {
      const axis = `zh分歧=${g.zhDiverges ? "Y" : "n"} ja/de分歧=${g.jaDeDiverges ? "Y" : "n"}`;
      console.log(`\n[${g.cls}] en=${JSON.stringify(g.en)}  ${axis}`);
      for (const v of g.variants) {
        console.log(`  zh=${JSON.stringify(v.values.zh)} ja=${JSON.stringify(v.values.ja)} de=${JSON.stringify(v.values.de)}`);
        for (const k of v.keys) console.log(`    ${k}`);
      }
    }
    console.log(
      `\n# driftGroups=${r.drift.length}（A=${r.drift.filter((g) => g.cls === "A").length} ` +
        `B=${r.drift.filter((g) => g.cls === "B").length} C=${r.drift.filter((g) => g.cls === "C").length}；` +
        `其中 ja/de 分歧 ${r.drift.filter((g) => g.jaDeDiverges).length}，zh 分歧 ${r.drift.filter((g) => g.zhDiverges).length}）`,
    );
    return;
  }

  if (argv.includes("--plural")) {
    for (const c of r.plural) {
      console.log(`\n${c.key}  [${c.flagged.join(",")}]  en=${JSON.stringify(c.en)}`);
      for (const l of LANGS) console.log(`  count=1 ${l}=${JSON.stringify(c.renders[l])}`);
    }
    console.log(`\n# countKeys=${r.packs.keys.filter((k) => r.packs.values.en[k].includes("{count}")).length} pluralCandidates=${r.plural.length}`);
    return;
  }

  if (argv.includes("--orphans")) {
    for (const o of r.orphans) console.log(o);
    console.log(`\n# orphans=${r.orphans.length}`);
    console.log("# [豁免] 以下键无 src/ 引用，但引用形式结构上不可能被字面量判据命中，逐条列出依据：");
    for (const e of r.exempt) console.log(`#   ${e.key}  —— ${e.reason}`);
    const changelog = r.packs.keys.filter((k) => k.startsWith("app.changelog."));
    const changelogSources = collectSources(root);
    const changelogReferenced = changelog.filter((k) => findReferences(k, changelogSources).length);
    console.log(
      `# [记录] app.changelog.* 无需豁免：src/changelog.ts 的 changeKeys 以字面量存 key，引用判据已能命中` +
        `（实测 ${changelogReferenced.length}/${changelog.length} 键被引用，无孤儿）`,
    );
    return;
  }

  const termIdx = argv.indexOf("--term");
  if (termIdx !== -1) {
    const pattern = argv[termIdx + 1];
    if (!pattern) {
      console.error("用法：node scripts/i18n-locale-scan.mjs --term <词形或正则>");
      process.exit(1);
    }
    const hits = findTerm(r.packs, pattern);
    for (const h of hits) {
      console.log(`\n${h.key}  [${h.hits.join(",")}]`);
      for (const l of h.hits) console.log(`  ${l}=${JSON.stringify(h.values[l])}`);
    }
    console.log(`\n# term=${pattern} keys=${hits.length}`);
    return;
  }

  console.error("用法：node scripts/i18n-locale-scan.mjs --duplicates|--drift|--plural|--orphans|--summary|--term <词形>");
  process.exit(1);
}

// 用 realpath 比较：经由软链接路径调用时 argv[1] 与 import.meta.url 的真身不同，
// 直接比较会让 main() 静默不执行——工具一声不响地退出 0，比不跑更糟。
if (process.argv[1] && realpathSyncSafe(process.argv[1]) === realpathSyncSafe(fileURLToPath(import.meta.url))) {
  main();
}

/** `realpathSync` 的容错包装（文件不存在时返回原串，不抛）。 */
function realpathSyncSafe(p) {
  try {
    return realpathSync(p);
  } catch {
    return p;
  }
}
