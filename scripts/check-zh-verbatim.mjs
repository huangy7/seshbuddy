#!/usr/bin/env node
// zh 逐字校验：把「这个键的 zh 该不该翻」从判断题变成可判定的字符串测试。
//
// 判据：某个键的 `en` 取值若在【该键所属文件的 base 版本】里逐字出现，说明那句英文
// 就是迁移前的原文，于是 `zh` 必须逐字等于 `en`。迁移的总规则是「zh 恒取组件内原文」，
// 原文是英文的站点，zh 就保持英文——「要翻」管的是 ja/de。
//
// ⚠️ **这是候选生成器，不是判定程序。** 判据的假阳性率很高，而且每一类假阳性都朝同一个
// 方向错——**把正确的中文 zh 判成错，逼执行者改成英文**，正是本次迁移要防的那件事。
// 所以每一条命中都必须逐条裁定，**不能拿它的退出码当结论**。
//
// 本仓实测过的假阳性来源（逐条核对过，无一为真）：
//
//   | 来源 | 例子 |
//   |---|---|
//   | 标识符片段 | `Selected` 命中 `exportSelected`，`Edit` 命中 `startEdit` |
//   | 别的文件里的同一个词 | `Cumulative` 命中 `SessionAnalyticsModal.vue` |
//   | 注释 | `<!-- API Key -->`、`// ─── Changes ───` |
//   | 测试断言 en 渲染 | `setLocale("en")` 后的 `.toBe("Select…")`，循环论证 |
//   | 键名自身 | `t('app.batch.selected')` 里含有 `selected` |
//   | 语言关键字 / 类型名 | `as unknown as`、`const min =`、`min-width` |
//   | 枚举值 / 键盘键名 | `s.status === "idle"`、`e.key === "Delete"` |
//   | 身份值（本项目明确不动） | `OFFICIAL_PROFILE_NAME = "Codex Official"` |
//   | CSS 自定义属性 | `var(--color-surface-selected)` |
//   | 片段相距任意远也命中 | 各静态片段由 `[\s\S]*` 连接，可跨任意距离（含跨行）——`Cache read: {count} ({rate}% hit)` 会命中「`Cache read:` …几千字符… `(% hit)`」 |
//
// 其中可机械判定的五类已经过滤掉（词边界、所属文件、注释、测试文件、键名自身），其余仍需人读。
// **不要再往上加过滤器**：每加一个过滤器就是加一个事实来源，而本仓的教训是
// 第二个事实来源正是缺陷的住处。残留的这几类靠报告里逐条裁定，比靠正则可靠。
// 最后一类（片段距离）同理**不加距离上限**：那是一个需要自己验证的新启发式，
// 与「候选生成器 + 人裁定」的定位不符，加了就得再验证它自己。
//
// ⚠️ **假阴性纪律：base 必须早于该键自己的迁移。** 用**批次的** base 对本批自己的键是对的，
// 对**更早批次遗留的键是错的**——那些键的所属文件在批次 base 里**已经是 `t()` 调用**，
// 原文早已消失，于是**结构上不可能命中**，而报告只会显示「无候选」，正是本仓反复付代价的
// 那种「跳过看起来像通过」。实证：`common.chatView.svgImage` 的原文在 `ChatView.vue`，
// 被更早的批次（`02558473`）迁走；用计划 4 的 base（`ae4dcfbb`）跑，`ChatView.vue` 那处
// 已是 `t("common.chatView.svgImage")`，命中不了；改用 `02558473^` 后 `ChatView.vue` 立刻命中。
// （这条键今天在 `ae4dcfbb` 下也会报出候选，但报的是**本批新迁的** `ChatSvgWidgetCard.vue`，
// 不是 `ChatView.vue`——能不能命中由所属文件决定，这不是反例。）
// → 跨批次核验时**逐键取「该键进入语言包的那个提交的父提交」**，不要用批次 base。
//
// 五条过滤各自的来历（每一条都由实测的失效形状倒推出来）：
//
// 1. **词边界匹配，不用子串。** 93 键的真实语料上，子串判据有 11 个键被标识符片段击中。
// 2. **限定在所属文件，不做全树扫描。** 全树版会把 `累计`/`编辑`/`删除` 这类正确译文判为 FAIL。
// 3. **先剥注释再匹配。** 注释里的英文与该键的原文无关，是全仓最常见的一类假命中。
// 4. **测试文件不算所属文件。** 迁移后新增的测试会断言 en 取值，那种出现是循环论证。
// 5. **抹掉键名自身。** 否则 `t('app.batch.selected')` 会被 en 取值 `{count} selected` 自我命中。
//
// 用法：node scripts/check-zh-verbatim.mjs <base-sha>
// 退出码：0 无候选 / 1 有候选待逐条裁定 / 2 一个候选都没有（需阳性对照）
import { readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { stripJsComments, flattenMessages as flattenPack } from "./check-i18n.mjs";

/**
 * 把 en 取值按 `{占位符}` 骨架切开，返回各静态片段。
 *
 * 原文常是模板字符串（`` `Thought for ${x}` ``），而语言包里是 `Thought for {duration}`，
 * 两者逐字比较必然不等，所以要比的是**静态片段**。
 * 长度 ≤ 1 的片段（多为占位符之间的空格）不参与匹配：它们在任何文件里都能命中，是纯噪声。
 */
export function splitFragments(en) {
  return en
    .split(/\{[^}]*\}/)
    .map((s) => s.trim())
    .filter((s) => s.length > 1);
}

/**
 * 构造该 en 取值的逐字匹配式；无可用片段时返回 null。
 *
 * 两侧用 `(?<![\w])` / `(?![\w])` 卡词边界——这是与子串判据的唯一区别，也是本工具存在的理由。
 * 片段之间允许任意间隔（含跨行），以覆盖多行模板。
 */
export function buildVerbatimRegex(en) {
  const frags = splitFragments(en);
  if (!frags.length) return null;
  const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const body = frags.map((f) => `(?<![\\w])${esc(f)}(?![\\w])`).join("[\\s\\S]*");
  return new RegExp(body, "m");
}

/**
 * 键的所属文件：当前工作树里引用了 `t("<键>")` 的**非测试**源文件。
 *
 * 不用「按目录猜」也不用「按任务清单记」——两者都会漂移。引用点就是唯一的事实来源，
 * 且它随迁移自然出现：一个键被迁移到哪个文件，那个文件就是它的所属文件。
 * 测试文件排除在外：迁移后新增的测试会断言 en 取值，那种出现是循环论证。
 */
export function findOwnerFiles(key, sources) {
  const doubleQuoted = `"${key}"`;
  const singleQuoted = `'${key}'`;
  return Object.keys(sources).filter(
    (f) =>
      !/\.test\.(ts|js|vue)$/.test(f) &&
      (sources[f].includes(doubleQuoted) || sources[f].includes(singleQuoted)),
  );
}

/**
 * 剥掉注释后再匹配：注释里的英文与该键的原文无关。
 *
 * 复用闸门的 `stripJsComments`（它同时处理 `//` 与 `/* *\/`），另外先去掉 Vue 模板注释——
 * 那是 HTML 语法，闸门那个函数不管。
 * **不复用就是在造第二个事实来源**，而剥注释这件事本仓已经因为「正则在字符串边界上失灵」
 * 吃过一次代价、才换成逐字符扫描的。
 */
export function stripComments(src) {
  return stripJsComments(src.replace(/<!--[\s\S]*?-->/g, ""));
}

/** 收集当前工作树里所有可能引用文案键的源文件。 */
function collectSources(root) {
  const out = {};
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) walk(p);
      else if (/\.(vue|ts)$/.test(name)) out[relative(root, p)] = readFileSync(p, "utf8");
    }
  };
  walk(join(root, "src"));
  return out;
}

/**
 * 执行一次 sweep。
 *
 * 读盘函数与源文件集合都由调用方注入：测试要能在不建 git 仓库、不碰真实语言包的情况下跑。
 * `sources` 尤其不能省——若它取自真实工作树，测试里的键在真实源码里找不到引用点，
 * 于是每个用例都因「无所属文件」而空过，测试全绿却什么都没验。
 */
export function sweep({ locales, readCurrent, readBase, sources = collectSources(process.cwd()) }) {
  const baseCache = new Map();
  const baseContent = (f) => {
    if (!baseCache.has(f)) baseCache.set(f, readBase(f));
    return baseCache.get(f);
  };

  const hits = [];
  const failures = [];
  let scanned = 0;
  let noOwner = 0;

  for (const ns of locales) {
    const en = JSON.parse(readCurrent(`src/locales/en/${ns}.json`));
    const zh = JSON.parse(readCurrent(`src/locales/zh/${ns}.json`));
    const zhMap = new Map(flattenPack(zh));

    for (const [key, value] of flattenPack(en)) {
      const full = `${ns}.${key}`;
      const owners = findOwnerFiles(full, sources);
      // 无所属文件的键单独计数并报出来：静默跳过是「覆盖率看起来 100%」的成因，
      // 而本仓反复付代价的正是这种「跳过看起来像通过」。
      if (!owners.length) {
        noOwner++;
        continue;
      }
      scanned++;

      const re = buildVerbatimRegex(value);
      if (!re) continue;

      // 键名自身也要抹掉：`t('app.batch.selected')` 里就含有 `selected`，
      // 而 en 取值 `{count} selected` 的静态片段正是 `selected`——不抹掉就是自我命中。
      const matched = owners.filter((f) => {
        const base = baseContent(f);
        if (base === null) return false;
        return re.test(stripComments(base).split(full).join(" "));
      });
      if (!matched.length) continue;

      const zhValue = zhMap.get(key);
      const ok = zhValue === value;
      hits.push({ key: full, en: value, zh: zhValue, files: matched });
      if (!ok) failures.push({ key: full, en: value, zh: zhValue, files: matched });
    }
  }

  return { scanned, noOwner, hits, failures };
}

function main() {
  const base = process.argv[2];
  if (!base) {
    console.error("用法：node scripts/check-zh-verbatim.mjs <base-sha>");
    process.exit(1);
  }

  const root = process.cwd();
  const readBase = (f) => {
    try {
      return execFileSync("git", ["show", `${base}:${f}`], {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      });
    } catch {
      return null; // base 里不存在该文件（本批新增）——无原文可言，不构成命中
    }
  };

  // `_meta.json` 是语言包的元数据（`reviewed` / `translatedFrom` / `reviewer`），不是命名空间：
  // 它没有消息键，若被当成命名空间，那三个字段会全部计进「无引用点跳过」的计数，
  // 使覆盖行的跳过数虚高。闸门规则 3 也是明确排除它的。
  const locales = readdirSync(join(root, "src/locales/en"))
    .filter((f) => f.endsWith(".json") && f !== "_meta.json")
    .map((f) => f.replace(/\.json$/, ""));

  const { scanned, noOwner, hits, failures } = sweep({
    locales,
    readCurrent: (p) => readFileSync(join(root, p), "utf8"),
    readBase,
  });

  for (const h of hits) {
    const mark = h.zh === h.en ? "OK  " : "✗   ";
    console.log(`${mark} ${h.key}\n     en=${JSON.stringify(h.en)}  zh=${JSON.stringify(h.zh)}\n     ${h.files.join(", ")}`);
  }

  console.log(
    `\n覆盖：有 t() 引用点、进入判据的键 ${scanned} 个；无引用点被跳过的键 ${noOwner} 个；候选 ${hits.length} 个。`,
  );

  if (failures.length) {
    console.error(
      `\n${failures.length} 个候选待裁定——en 在 base 里逐字出现，zh 却不等于 en。\n` +
        `这是候选而非结论：判据的假阳性会把正确的中文判成错，逐条读完再决定改不改。\n` +
        failures.map((f) => `  ✗ ${f.key}: en=${JSON.stringify(f.en)} zh=${JSON.stringify(f.zh)}`).join("\n"),
    );
    process.exit(1);
  }

  if (hits.length === 0) {
    console.error(
      "一个候选都没有——这与「检查坏了」在输出上无法区分，故不当作通过。\n" +
        "先做阳性对照：拿一个已知的英文原文站点（例如 base 里的 `Thinking…`）跑同一条命令，\n" +
        "确认它确实报出命中，再回来看这个 0。",
    );
    process.exit(2);
  }
}

// 用 realpath 比较：经由软链接路径调用时（macOS 的 /tmp、被链接的检出目录），
// argv[1] 的写法与 import.meta.url 解析出的真身不同，直接比较会让 main() 静默不执行——
// 工具一声不响地退出 0，比不跑更糟。
if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
) {
  main();
}
