#!/usr/bin/env node
/**
 * 规则 9 允许清单的【机械派生】：`scripts/i18n-drift-adjudication.json`
 * → `scripts/i18n-drift-allowlist.json`。
 *
 * **两个文件不是两个事实来源。** 裁决集按 `en` 值记（人能读、能追溯到「为什么这是两条
 * 不同的源文案」），允许清单是闸门能查的投影。**清单不要手写**：只改清单等于把一次裁定
 * 藏进闸门的数据里，下一个做测量的人不会知道它已经被裁过。**新增条目要先写进裁决集**，
 * 再跑本脚本派生过来。
 *
 * ## 只收「规则 9 真会命中」的条目（Ruling 10）
 *
 * 裁决集里 `decision === "keep"` 的每条**不必然**进清单：`keep` 只是说「这不是译文没对齐」，
 * 而规则 9 的判据是「同一 `en` 值下 `ja`/`de` 取值不唯一」。**`keep` 但 ja/de 本来就一致的
 * 条目永远不会被规则 9 命中**（它们的 `zh` 分歧不参与判据），把它们列进去只会让清单看起来
 * 比实际抓力大——**故由本脚本过滤掉，并打印「考虑了几条 / 发出了几条」两个计数**。
 * 实测的两个实例是 `Loading…` 与 `Saving…`：T6 合并后它们的 ja/de 逐字相同，
 * 分歧只在 `zh`。
 *
 * ## 判据只有一份实现
 *
 * 过滤用的 `findUnlistedDrift` 与语言包的展平 `flattenMessages` **都从 `check-i18n.mjs` 导入**
 * ——那正是闸门自己跑的那一份。本脚本不另写一份判据：两处实现一旦分叉，清单里的条目会与
 * 闸门实际取到的 `en` 值对不上，逐条落空而输出看起来只是「清单里某条没用了」。
 *
 * 用法：
 *   node scripts/gen-drift-allowlist.mjs            # 派生并写回清单
 *   node scripts/gen-drift-allowlist.mjs --dry-run  # 只打印，不写文件
 */
import { readFileSync, readdirSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { DRIFT_LANGS, findUnlistedDrift, flattenMessages } from "./check-i18n.mjs";

/** `src/locales/en/` 下的命名空间清单（`_meta.json` 是元数据，不是命名空间）。 */
function listNamespaces(root) {
  return readdirSync(join(root, "src/locales/en"))
    .filter((f) => f.endsWith(".json") && f !== "_meta.json")
    .sort();
}

/** 三语 × 全部命名空间，展平成 `ns.key → 文本`（键口径与闸门规则 3 一致）。 */
function loadDriftPacks(root, namespaces) {
  const packs = {};
  for (const lang of DRIFT_LANGS) {
    packs[lang] = {};
    for (const ns of namespaces) {
      const name = ns.replace(/\.json$/, "");
      const pack = JSON.parse(readFileSync(join(root, `src/locales/${lang}/${ns}`), "utf8"));
      for (const [k, v] of flattenMessages(pack)) packs[lang][`${name}.${k}`] = v;
    }
  }
  return packs;
}

/** 理由的首句（`：` 之前），用作「理由分布」的分组标签。 */
function reasonLabel(reason) {
  const head = String(reason).split("：")[0].trim();
  return head.length > 40 ? `${head.slice(0, 40)}…` : head;
}

/**
 * 机械派生允许清单：读 `root` 下的裁决集与语言包，返回
 * `{ allowlist, emitted, skipped, keeps }`（`allowlist` 即要写回文件的那个对象）。
 *
 * 导出它是为了让**「提交的清单 == 裁决集的投影」这条等式有一个会失败的检查**
 * （`scripts/gen-drift-allowlist.test.mjs`）：派生逻辑只此一份，测试与生成脚本调用同一函数，
 * 不另写一份判据。`root` 可传夹具目录，故阳性对照（扰动一条 `reason`）也能跑同一条代码路径。
 */
export function deriveAllowlist(root) {
  const namespaces = listNamespaces(root);
  const adjudication = JSON.parse(
    readFileSync(join(root, "scripts/i18n-drift-adjudication.json"), "utf8")
  );

  const keeps = adjudication.drift.filter((e) => e.decision === "keep");
  // 空允许清单 ⇒ 本函数返回**全部**会命中的 `en` 值，即「规则 9 今天真会命中的集合」。
  const firing = new Set(findUnlistedDrift(loadDriftPacks(root, namespaces), {}).map((h) => h.en));

  const emitted = keeps.filter((e) => firing.has(e.en));
  const skipped = keeps.filter((e) => !firing.has(e.en));

  const allowlist = {};
  for (const entry of [...emitted].sort((a, b) => a.en.localeCompare(b.en))) {
    allowlist[entry.en] = entry.reason;
  }
  return { allowlist, emitted, skipped, keeps };
}

function main() {
  const root = fileURLToPath(new URL("..", import.meta.url));
  const namespaces = listNamespaces(root);
  const { allowlist, emitted, skipped, keeps } = deriveAllowlist(root);

  console.log(`命名空间 ${namespaces.length} 个；裁决集 keep 条目 ${keeps.length} 条（考虑）`);
  console.log(
    `其中 ja/de 确有分歧、规则 9 真会命中 ${emitted.length} 条（发出）；` +
      `ja/de 本来就一致、规则 9 永不命中 ${skipped.length} 条（不发）`
  );
  const labels = {};
  for (const entry of emitted) {
    const label = reasonLabel(entry.reason);
    labels[label] = (labels[label] ?? 0) + 1;
  }
  console.log("发出的条目按理由分布：");
  for (const [label, n] of Object.entries(labels).sort((a, b) => b[1] - a[1])) {
    console.log(`  ${n}\t${label}`);
  }
  if (skipped.length) {
    console.log("不发（规则 9 永不命中）的条目，逐条列出供核对：");
    for (const entry of skipped) console.log(`  ${JSON.stringify(entry.en)}`);
  }

  if (process.argv.includes("--dry-run")) return;
  writeFileSync(
    join(root, "scripts/i18n-drift-allowlist.json"),
    `${JSON.stringify(allowlist, null, 2)}\n`
  );
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  main();
}
