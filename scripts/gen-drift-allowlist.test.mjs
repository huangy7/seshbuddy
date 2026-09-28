import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { findUnlistedDrift, flattenMessages } from "./check-i18n.mjs";
import { deriveAllowlist } from "./gen-drift-allowlist.mjs";

/**
 * 规则 9 的允许清单是**裁决集的机器可读投影，不是第二份事实来源**——这条设计成立的前提是
 * 「提交的清单 == 由裁决集派生的结果」。此前没有任何检查守着这个等式：改了
 * `scripts/i18n-drift-adjudication.json`（比如订正一条 `reason`）而忘了重跑生成脚本，
 * 闸门里的理由文本就静默过期，而一切照常是绿的——**失败是单向且无声的**。
 *
 * 本文件的等值断言用的是生成脚本自己的 `deriveAllowlist`（它内部调用的正是闸门的
 * `findUnlistedDrift` / `flattenMessages`），不另写一份判据——两份判据一旦分叉，
 * 清单里的条目会与闸门实际取到的 `en` 值对不上，逐条落空而输出看起来只是「某条没用了」。
 *
 * 每个断言都配**阳性对照**：只断言「两边相等」时，一个恒返回 `{}` 的派生与一份真的空清单
 * 在输出上无法区分。
 */

const ROOT = process.cwd();
const ALLOWLIST = join(ROOT, "scripts/i18n-drift-allowlist.json");

const tempRoots = [];
/** 把真实的语言包与裁决集复制进夹具目录：阳性对照要跑**同一条代码路径**。 */
function fixtureRootFromRepo() {
  const root = mkdtempSync(join(tmpdir(), "drift-allowlist-"));
  tempRoots.push(root);
  cpSync(join(ROOT, "src/locales"), join(root, "src/locales"), { recursive: true });
  mkdirSync(join(root, "scripts"), { recursive: true });
  cpSync(
    join(ROOT, "scripts/i18n-drift-adjudication.json"),
    join(root, "scripts/i18n-drift-adjudication.json")
  );
  return root;
}
afterAll(() => {
  for (const r of tempRoots) rmSync(r, { recursive: true, force: true });
});

describe("gen-drift-allowlist：允许清单是裁决集的投影", () => {
  it("提交的 scripts/i18n-drift-allowlist.json 等于在内存里派生的结果", () => {
    const { allowlist, keeps, emitted, skipped } = deriveAllowlist(ROOT);
    // 阳性对照一：派生确实读到了裁决集与语言包——否则「两边都是空」也会相等
    expect(keeps.length).toBeGreaterThan(0);
    expect(emitted.length).toBeGreaterThan(0);
    expect(skipped.length).toBeGreaterThan(0);
    expect(Object.keys(allowlist).length).toBeGreaterThan(0);
    // 阳性对照二：发出的条目确实是 keep 的子集（口径是「keep 且规则 9 真会命中」）
    const keepEns = new Set(keeps.map((e) => e.en));
    expect(Object.keys(allowlist).filter((en) => !keepEns.has(en))).toEqual([]);

    expect(JSON.parse(readFileSync(ALLOWLIST, "utf8"))).toEqual(allowlist);
  });

  it("阳性对照：扰动裁决集里一条 reason，等值断言即失效", () => {
    const committed = JSON.parse(readFileSync(ALLOWLIST, "utf8"));
    const targetEn = Object.keys(committed)[0];
    const root = fixtureRootFromRepo();
    const adjPath = join(root, "scripts/i18n-drift-adjudication.json");
    const adj = JSON.parse(readFileSync(adjPath, "utf8"));
    const entry = adj.drift.find((d) => d.en === targetEn);
    expect(entry, `夹具里找不到 en=${JSON.stringify(targetEn)} 的裁决条目`).toBeTruthy();
    entry.reason = `${entry.reason}（夹具扰动）`;
    writeFileSync(adjPath, JSON.stringify(adj, null, 2));

    const { allowlist } = deriveAllowlist(root);
    expect(allowlist).not.toEqual(committed);
    // 差异**只**落在被扰动的那条上：否则这条对照可能在测别的东西
    expect(Object.keys(committed).filter((k) => committed[k] !== allowlist[k])).toEqual([targetEn]);
  });

  it("判据有抓力：闸门的 findUnlistedDrift 在空清单下能报出漂移组", () => {
    // 用闸门的展平函数造夹具（键口径与生成脚本一致），再喂闸门的判据
    const packs = {
      en: Object.fromEntries(flattenMessages({ app: { a: "Loading…", b: "Loading…" } })),
      ja: Object.fromEntries(flattenMessages({ app: { a: "読み込み中…", b: "読み込み中..." } })),
      de: Object.fromEntries(flattenMessages({ app: { a: "Wird geladen…", b: "Wird geladen..." } })),
    };
    expect(findUnlistedDrift(packs, {}).map((h) => h.en)).toEqual(["Loading…"]);
    // 反面：同一份夹具，该 en 值进了允许清单即不报——这正是「清单消化裁定」的机制
    expect(findUnlistedDrift(packs, { "Loading…": "夹具里已裁定" })).toEqual([]);
  });
});
