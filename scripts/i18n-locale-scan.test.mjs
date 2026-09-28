import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
  buildI18n,
  collectSources,
  ellipsisForm,
  findDuplicateGroups,
  findDriftGroups,
  findEllipsisFormDivergences,
  findOrphans,
  findPluralCandidates,
  findReferences,
  findRustNativeTextKeys,
  findTerm,
  flattenPack,
  listNamespaces,
  loadPacks,
  measure,
  renderCount,
  tupleOf,
} from "./i18n-locale-scan.mjs";

/**
 * 每个夹具断言都配一条**阳性对照**。
 *
 * 阳性对照验证的是【工具】，不是【覆盖】：证明「判据在能命中时确实命中」。
 * 少了它，一个「永远返回空」的坏判据与一个「真的干净」的输入在输出上无法区分——
 * 本仓已经在这上面翻过两次车。
 */

const ROOT = process.cwd();
const LANGS = ["en", "zh", "ja", "de"];

const tmpRoots = [];
function fixtureRoot(files) {
  const root = mkdtempSync(join(tmpdir(), "locale-scan-"));
  tmpRoots.push(root);
  for (const [rel, body] of Object.entries(files)) {
    const full = join(root, rel);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, typeof body === "string" ? body : JSON.stringify(body));
  }
  return root;
}
afterAll(() => {
  for (const r of tmpRoots) rmSync(r, { recursive: true, force: true });
});

/** 四语同键的夹具包：`{ "<ns>.<key>": { en, zh, ja, de } }` → `{en: {ns: {...}}, …}`。 */
function messagesFrom(spec) {
  const out = Object.fromEntries(LANGS.map((l) => [l, {}]));
  for (const [full, vals] of Object.entries(spec)) {
    const [ns, ...rest] = full.split(".");
    for (const l of LANGS) {
      out[l][ns] ??= {};
      out[l][ns][rest.join(".")] = vals[l];
    }
  }
  return out;
}

/** 把夹具包落盘成真实语言包目录，供 `loadPacks` / `measure` 用。 */
function localeFiles(spec, extra = {}) {
  const files = {};
  for (const l of LANGS) {
    const byNs = {};
    for (const [full, vals] of Object.entries(spec)) {
      const [ns, ...rest] = full.split(".");
      byNs[ns] ??= {};
      byNs[ns][rest.join(".")] = vals[l];
    }
    for (const [ns, pack] of Object.entries(byNs)) files[`src/locales/${l}/${ns}.json`] = pack;
  }
  return { ...files, ...extra };
}

describe("listNamespaces / flattenPack / loadPacks", () => {
  it("命名空间从 en/ 目录派生，排除 _meta.json", () => {
    const root = fixtureRoot({
      "src/locales/en/_meta.json": { reviewed: false },
      "src/locales/en/app.json": { a: "A" },
      "src/locales/en/chat.json": { b: { c: "B" } },
    });
    expect(listNamespaces(root)).toEqual(["app", "chat"]);
  });

  it("嵌套对象展平成点分键；loadPacks 取到四语取值", () => {
    const root = fixtureRoot(
      localeFiles({
        "app.deep.key": { en: "A", zh: "甲", ja: "ア", de: "A-de" },
      }),
    );
    const packs = loadPacks(root);
    expect(packs.keys).toEqual(["app.deep.key"]);
    expect(tupleOf(packs.values, "app.deep.key")).toEqual(["A", "甲", "ア", "A-de"]);
    // 阳性对照：展平器确实按 `.` 连接嵌套层，而不是只取顶层键
    expect(flattenPack({ deep: { key: "A" } })).toEqual([["deep.key", "A"]]);
  });
});

describe("--duplicates：元组口径（不是「四语全等」）", () => {
  // ①a 四语全等（标识符形态）与 ①b 元组相同但四语不全等——**两者都必须被抓到**。
  const spec = {
    "app.sameAllLangs": { en: "Headers", zh: "Headers", ja: "Headers", de: "Headers" },
    "app.sameAllLangsTwin": { en: "Headers", zh: "Headers", ja: "Headers", de: "Headers" },
    "app.sameTuple": { en: "Close", zh: "关闭", ja: "閉じる", de: "Schließen" },
    "app.sameTupleTwin": { en: "Close", zh: "关闭", ja: "閉じる", de: "Schließen" },
    "app.unique": { en: "Only", zh: "唯一", ja: "唯一", de: "Einzig" },
    "app.emptyOne": { en: "", zh: "", ja: "", de: "" },
    "app.emptyTwo": { en: "", zh: "", ja: "", de: "" },
  };

  it("四语全等与元组相同都算重复；取值唯一的键不算", () => {
    const groups = findDuplicateGroups(loadPacks(fixtureRoot(localeFiles(spec))));
    const byKey = Object.fromEntries(groups.map((g) => [g.keys[0], g]));
    expect(byKey["app.sameAllLangs"].keys).toEqual(["app.sameAllLangs", "app.sameAllLangsTwin"]);
    expect(byKey["app.sameAllLangs"].allLangsEqual).toBe(true);
    expect(byKey["app.sameTuple"].keys).toEqual(["app.sameTuple", "app.sameTupleTwin"]);
    expect(byKey["app.sameTuple"].allLangsEqual).toBe(false);
    // 阳性对照的反面：取值唯一的键不构成组
    expect(groups.some((g) => g.keys.includes("app.unique"))).toBe(false);
    // 全空值组被标记出来（92/93 之差全在这一类）
    expect(byKey["app.emptyOne"].allEmpty).toBe(true);
    expect(byKey["app.sameTuple"].allEmpty).toBe(false);
  });

  it("元组口径与「四语全等」口径确实不同——同一份夹具下两者组数不等", () => {
    const groups = findDuplicateGroups(loadPacks(fixtureRoot(localeFiles(spec))));
    expect(groups.length).toBe(3);
    // 「四语全等」口径只认标识符形态那一组（外加全空值组——空串也是四语全等）
    expect(groups.filter((g) => g.allLangsEqual && !g.allEmpty).length).toBe(1);
  });
});

describe("--drift：同一 en、元组不同", () => {
  const spec = {
    "app.a": { en: "Loading…", zh: "加载中...", ja: "読み込み中...", de: "Wird geladen..." },
    "app.b": { en: "Loading…", zh: "加载中…", ja: "読み込み中…", de: "Wird geladen…" },
    "app.sameEnSameTuple": { en: "Close", zh: "关闭", ja: "閉じる", de: "Schließen" },
    "app.sameEnSameTupleTwin": { en: "Close", zh: "关闭", ja: "閉じる", de: "Schließen" },
  };

  it("同 en 不同元组 → 一个漂移组，两个变体", () => {
    const drift = findDriftGroups(loadPacks(fixtureRoot(localeFiles(spec))));
    const g = drift.find((x) => x.en === "Loading…");
    expect(g.variants.length).toBe(2);
    expect(g.jaDeDiverges).toBe(true);
    // 阳性对照：同 en **同元组**的那对是重复组，不是漂移组
    expect(drift.some((x) => x.en === "Close")).toBe(false);
    expect(findDuplicateGroups(loadPacks(fixtureRoot(localeFiles(spec)))).some((d) => d.keys.includes("app.sameEnSameTuple"))).toBe(true);
  });

  it("分类：A=一为英文原文一为中文原文；C=zh 一致、只有 ja/de 分歧", () => {
    const drift = findDriftGroups(
      loadPacks(
        fixtureRoot(
          localeFiles({
            // A：tabRequest 原文是英文（zh === en），request 原文是中文
            "app.tabRequest": { en: "Request", zh: "Request", ja: "リクエスト", de: "Anfrage" },
            "app.request": { en: "Request", zh: "请求", ja: "リクエスト", de: "Anfrage" },
            // C：zh 一致，ja/de 分歧
            "app.split": { en: "Splits: {count}", zh: "{count}分屏", ja: "{count} 分割", de: "Teilungen: {count}" },
            "app.splitCount": { en: "Splits: {count}", zh: "{count}分屏", ja: "{count}分割", de: "Bereiche: {count}" },
          }),
        ),
      ),
    );
    expect(drift.find((x) => x.en === "Request").cls).toBe("A");
    const c = drift.find((x) => x.en === "Splits: {count}");
    expect(c.cls).toBe("C");
    expect(c.zhDiverges).toBe(false);
    expect(c.jaDeDiverges).toBe(true);
  });
});

describe("--plural：真 vue-i18n 渲染", () => {
  const candidates = (spec) => findPluralCandidates(messagesFrom(spec));

  it("`{count} files` 报不安全；`Changes: {count}` 报安全（阳性对照）", () => {
    const hits = candidates({
      "app.files": { en: "{count} files", zh: "{count} 个文件", ja: "{count} ファイル", de: "{count} Dateien" },
      "app.changes": { en: "Changes: {count}", zh: "{count} 项变更", ja: "変更 {count}", de: "Änderungen: {count}" },
    });
    expect(hits.map((h) => h.key)).toEqual(["app.files"]);
    expect(hits[0].flagged).toContain("en");
    expect(hits[0].renders.en).toBe("1 files");
    // 阳性对照：同一实例在 count=2 下渲染出 `2 files`——证明渲染是真跑的，不是常量
    const i18n = buildI18n(messagesFrom({ "app.files": { en: "{count} files", zh: "", ja: "", de: "" } }));
    expect(renderCount(i18n, "app.files", "en", 2)).toBe("2 files");
    expect(renderCount(i18n, "app.files", "en", 1)).toBe("1 files");
  });

  it("`|`-form 已存在的视为安全——判据是渲染结果，不是正则豁免", () => {
    const hits = candidates({
      "app.files": { en: "{count} file | {count} files", zh: "{count} 个文件", ja: "{count} ファイル", de: "{count} Datei | {count} Dateien" },
    });
    expect(hits).toEqual([]);
    // 阳性对照：真渲染确实选到了单数分支（否则「安全」可能只是没渲染）
    const i18n = buildI18n(messagesFrom({ "app.files": { en: "{count} file | {count} files", zh: "", ja: "", de: "" } }));
    expect(renderCount(i18n, "app.files", "en", 1)).toBe("1 file");
    expect(renderCount(i18n, "app.files", "en", 3)).toBe("3 files");
  });

  it("de 的字面 `(en)` 对冲写法被标出，且只在 de 上标", () => {
    const hits = candidates({
      "app.days": { en: "{count} d ago", zh: "{count}天前", ja: "{count}日前", de: "vor {count} Tag(en)" },
    });
    expect(hits.length).toBe(1);
    expect(hits[0].flagged).toEqual(["de"]);
  });

  it("无复数屈折的语言不单独构成候选：de 的形态不套到 en 的渲染上", () => {
    const hits = candidates({
      "app.idle": { en: "{count} idle", zh: "{count} 空闲", ja: "{count} アイドル", de: "{count} im Leerlauf" },
    });
    expect(hits).toEqual([]);
  });
});

describe("--orphans：引用判据与豁免", () => {
  // 引用表走真实的 `collectSources`（排除 `*.test.ts`），不手工喂一份——否则
  // 「测试文件不算引用」这条判据在夹具里根本没被执行到，用例会空过。
  const sources = collectSources(
    fixtureRoot({
      "src/used.ts": 'const a = t("app.used");\n',
      "src/used.test.ts": 'expect(t("app.onlyInTest")).toBe("x");\n',
      "src/prefix.ts": 'const b = t("app.history.title");\n',
    }),
  );
  const keys = ["app.used", "app.onlyInTest", "app.history", "app.history.title", "app.dead"];

  it("无人引用的键报孤儿；在用的键不报（含阳性对照）", () => {
    const { orphans } = findOrphans({ keys }, sources);
    expect(orphans).toContain("app.dead");
    expect(orphans).toContain("app.onlyInTest"); // 测试文件的引用不算
    expect(orphans).not.toContain("app.used");
    // 阳性对照：同一串放在非测试文件里就命中
    expect(findReferences("app.onlyInTest", { "src/used.ts": 'const a = t("app.onlyInTest");\n' })).toEqual(["src/used.ts"]);
    // 阳性对照：`collectSources` 确实把测试文件排除了，而不是从来没看见它
    expect(findReferences("app.onlyInTest", collectSources(fixtureRoot({ "src/x.test.ts": 't("app.onlyInTest");' })))).toEqual([]);
  });

  it("前缀键不被后缀键的引用遮住——用引号形态而不是裸子串", () => {
    const { orphans } = findOrphans({ keys }, sources);
    expect(orphans).toContain("app.history");
    expect(orphans).not.toContain("app.history.title");
  });

  it("动态拼键按具名规则豁免；前缀本身不豁免", () => {
    const { orphans, exempt } = findOrphans(
      { keys: ["assistant.phraseVariables.today", "cli.installHint.claude.windows", "cli.installHint.claude", "tray.quit"] },
      sources,
      { rustKeys: new Set(["tray.quit"]) },
    );
    expect(orphans).toEqual(["cli.installHint.claude"]);
    expect(exempt.map((e) => e.key).sort()).toEqual(["assistant.phraseVariables.today", "cli.installHint.claude.windows", "tray.quit"]);
    // 阳性对照：豁免规则各自带依据，且后缀规则只吃 `.windows` / `.unix`
    expect(exempt.every((e) => e.reason.length > 0)).toBe(true);
    expect(findOrphans({ keys: ["cli.installHint.claude.windows"] }, sources).exempt.length).toBe(1);
    expect(findOrphans({ keys: ["cli.installHint.claude.bsd"] }, sources).orphans).toEqual(["cli.installHint.claude.bsd"]);
  });
});

describe("findRustNativeTextKeys / collectSources / findTerm", () => {
  it("从 Rust 源码里机械提取 native_text 的 (ns, key)", () => {
    const root = fixtureRoot({
      "src-tauri/src/tray.rs": 'let a = native_text("tray", "quit");\nlet b = native_text_with("tray", "updateAvailable", "version", v);\n',
    });
    expect([...findRustNativeTextKeys(root)].sort()).toEqual(["tray.quit", "tray.updateAvailable"]);
  });

  it("collectSources 排除 *.test.ts", () => {
    const root = fixtureRoot({ "src/a.ts": "x", "src/b.vue": "y", "src/c.test.ts": "z" });
    expect(Object.keys(collectSources(root))).toEqual(["src/a.ts", "src/b.vue"]);
  });

  it("--term 给一个词形，列出匹配的键与语言", () => {
    const packs = loadPacks(
      fixtureRoot(
        localeFiles({
          "app.a": { en: "Clear", zh: "清除", ja: "クリア", de: "Leeren" },
          "app.b": { en: "Delete", zh: "删除", ja: "削除", de: "Löschen" },
        }),
      ),
    );
    expect(findTerm(packs, "Leeren").map((h) => h.key)).toEqual(["app.a"]);
    expect(findTerm(packs, "Löschen").map((h) => h.key)).toEqual(["app.b"]);
    // 阳性对照：一个都不匹配时返回空，说明上面两条不是「恒返回全部」
    expect(findTerm(packs, "存在しない")).toEqual([]);
  });
});

/**
 * 本用例测的是**仓库的当前状态**，不是脚本机制——本文件其余用例都在夹具上跑。
 *
 * **为什么必须有它**：裁决集是下游任务的唯一事实来源（T4/T5/T6 按它执行、T9 由它派生
 * 允许清单），而「脚本报出的候选」与「裁决集里的条目」是两份会各自漂移的东西。
 * 本用例把那层关系变成**一个会失败的检查**：每条候选要么已被处置，要么不在候选里，
 * **不允许第三类**。
 */
describe("仓库现状：候选与三份裁决集闭合", () => {
  const r = measure(ROOT);
  const readAdj = (f) => JSON.parse(readFileSync(join(ROOT, "scripts", f), "utf8"));

  it("每个重复组的每个键都有裁决条目，且 merge 的 into 指向同组内的键", () => {
    const adj = readAdj("i18n-dedup-adjudication.json");
    const missing = [];
    const badInto = [];
    const badSurvivor = [];
    for (const g of r.duplicates) {
      for (const k of g.keys) {
        const e = adj[k];
        if (!e) missing.push(k);
        else if (e.action === "merge") {
          if (!g.keys.includes(e.into)) badInto.push(`${k} → ${e.into}`);
          // survivor 自身必须是 keep：T5 对 action==="merge" 的键会「改调用点 + 删键」，
          // survivor 若也是 merge，它会被自己删掉。
          else if (adj[e.into]?.action !== "keep") badSurvivor.push(`${k} → ${e.into}（survivor action=${adj[e.into]?.action}）`);
        } else if (e.action !== "keep") badInto.push(`${k} action=${e.action}`);
      }
    }
    // 阳性对照：本次测量确实取到了重复组——否则「差集为空」与「扫描器恒返回空」无法区分
    expect(r.duplicates.length).toBeGreaterThan(0);
    expect(missing, `重复组里这些键没有裁决条目：\n${missing.join("\n")}`).toEqual([]);
    expect(badInto, `merge 的 into 不在同组内：\n${badInto.join("\n")}`).toEqual([]);
    expect(badSurvivor, `survivor 自身不是 keep：\n${badSurvivor.join("\n")}`).toEqual([]);
    // 反向：裁决集里的键都还在重复组里（键名口径，不按组号）
    const inGroups = new Set(r.duplicates.flatMap((g) => g.keys));
    expect(Object.keys(adj).filter((k) => !inGroups.has(k))).toEqual([]);
  });

  it("每条复数候选都有裁决条目，且带理由", () => {
    const adj = readAdj("i18n-plural-adjudication.json");
    const missing = r.plural.filter((c) => !adj[c.key]).map((c) => c.key);
    expect(r.plural.length).toBeGreaterThan(0);
    expect(missing, `这些复数候选没有裁决条目：\n${missing.join("\n")}`).toEqual([]);
    expect(Object.values(adj).every((e) => typeof e.reason === "string" && e.reason.length > 0)).toBe(true);
    expect(Object.values(adj).every((e) => ["pluralize", "keep", "delete"].includes(e.action))).toBe(true);
    expect(Object.values(adj).every((e) => typeof e.reachable === "boolean")).toBe(true);
  });

  /**
   * `pluralize` 项的反向判据：键在四语里都还在，且**已不再被工具的判据报成候选**。
   *
   * 正向只断言「候选 ⊆ 裁决集」，于是**裁决集这一侧没有任何检查**：键被改名、或被改回
   * 复数不安全的取值，都落在正向判据的盲区里——`pluralize` 条目会静默指着一个不存在的键，
   * 而唯一能发现它的是一次没人跑的手工重扫。规则 9 管的是译文漂移，管不到这里。
   *
   * ⚠️ **判据不是「`en` 取值含 `|`」**（那是对本项更早的一次描述）：`common.relativeTime.daysAgo`
   * 的 `en` 是 `{count} d ago`——英文的 `1 d ago` 无屈折问题，本条只把 **de** 改成
   * `vor {count} Tag | vor {count} Tagen`（见裁决集里该条 reason）。实测 5 条 `pluralize` 里
   * **4 条的 `en` 含 `|`、1 条不含**，故「含 `|`」会在正确的仓库状态上误报。
   * 改用「不在候选集里」：它走工具自己的判据，且**覆盖四语**——某一语（例如 de）回退成
   * 复数不安全同样会被抓住，而「`en` 含 `|`」只看 `en`，抓不到。
   */
  function pluralizeViolations(keys, packs, candidateKeys) {
    return keys.filter(
      (k) =>
        LANGS.some((l) => packs.values[l][k] === undefined) ||
        candidateKeys.has(k)
    );
  }

  it("反向：每条 pluralize 的键四语都在，且不再被报成复数候选（未回退）", () => {
    const adj = readAdj("i18n-plural-adjudication.json");
    const pluralizeKeys = Object.entries(adj)
      .filter(([, e]) => e.action === "pluralize")
      .map(([k]) => k);
    const candidateKeys = new Set(r.plural.map((c) => c.key));
    // 阳性对照一：两个集合都非空，否则下面的空集恒真
    expect(pluralizeKeys.length).toBeGreaterThan(0);
    expect(candidateKeys.size).toBeGreaterThan(0);
    expect(pluralizeViolations(pluralizeKeys, r.packs, candidateKeys)).toEqual([]);
    // 阳性对照二：同一判据在夹具上两种违规都报——缺键与「又被报成候选」各一例
    const fixture = {
      values: {
        en: { "app.ok": "x", "app.regressed": "y", "app.gone": "z" },
        zh: { "app.ok": "甲", "app.regressed": "乙", "app.gone": "丙" },
        ja: { "app.ok": "ア", "app.regressed": "イ" },
        de: { "app.ok": "A", "app.regressed": "B", "app.gone": "C" },
      },
    };
    expect(
      pluralizeViolations(["app.ok", "app.regressed", "app.gone"], fixture, new Set(["app.regressed"]))
    ).toEqual(["app.regressed", "app.gone"]);
  });

  it("每个漂移组的 en 取值都有裁决条目，且带理由", () => {
    const adj = readAdj("i18n-drift-adjudication.json");
    const byEn = new Map(adj.drift.map((d) => [d.en, d]));
    const missing = r.drift.filter((g) => !byEn.has(g.en)).map((g) => g.en);
    expect(r.drift.length).toBeGreaterThan(0);
    expect(missing, `这些漂移组没有裁决条目：\n${missing.join("\n")}`).toEqual([]);
    expect(adj.drift.every((d) => ["unify", "keep"].includes(d.decision) && d.reason.length > 0)).toBe(true);
    // 反向：`keep` 的 en 都还在漂移组里。`unify` 项**不**在此列——它一经应用，组内 ja/de
    // 就一致了，该组随即离开 --drift（这正是 unify 的目的，不是缺口）。
    const ens = new Set(r.drift.map((g) => g.en));
    expect(adj.drift.filter((d) => d.decision === "keep" && !ens.has(d.en)).map((d) => d.en)).toEqual([]);
    // 反面的反面：`unify` 项必须**已经应用**——它还留在漂移组里就说明 ja/de 没统一到位
    expect(adj.drift.filter((d) => d.decision === "unify" && ens.has(d.en)).map((d) => d.en)).toEqual([]);
    // 术语段：每个概念都要有 canonical、keys 与 reason
    expect(adj.terminology.length).toBeGreaterThan(0);
    // `keys` 是「**还需要改**的键」。省略号项在 T6 应用后判据输出为空，其 keys 也随之收敛为 []，
    // 故「非空」只对其余条目成立——「范围必须由判据派生」这条由下一个用例的等值检查守卫。
    expect(adj.terminology.every((t) => t.concept && Array.isArray(t.keys) && t.reason.length > 0)).toBe(true);
    expect(adj.terminology.filter((t) => !t.concept.startsWith("省略号")).every((t) => t.keys.length > 0)).toBe(true);
    // 术语段里点名的键必须真实存在（拼错键名会让统一项静默落空）
    const allKeys = new Set(r.packs.keys);
    const badKeys = adj.terminology.flatMap((t) => t.keys.filter((k) => !allKeys.has(k)));
    expect(badKeys, `术语段里这些键在语言包里不存在：\n${badKeys.join("\n")}`).toEqual([]);
  });

  // 术语项的**范围**必须由判据派生，不能手工圈定：手工范围会漏掉同类里其余成员，
  // 而条目本身又断言「就这些」——后来的人无从分辨那是范围选择还是漏了。
  it("省略号术语项的 keys 等于机械判据的输出（T6 应用后两边都为空）", () => {
    const adj = readAdj("i18n-drift-adjudication.json");
    const ell = adj.terminology.find((t) => t.concept.startsWith("省略号"));
    const derived = findEllipsisFormDivergences(r.packs);
    // T6 已按本项把 ja/de 的省略号形态逐键对齐到各自的 en，故判据输出与条目的 keys 都收敛为 []。
    expect(ell.keys, `裁决集与判据的差集：${derived.filter((k) => !ell.keys.includes(k)).join("、")}`).toEqual(derived);
    // ★ 阳性对照**换成夹具**：直接读仓时「判据恒返回空」与「仓库确实已收敛」在输出上无法区分。
    // 喂一个 ja 的形态与 en 不同的键，判据必须报出它——这才证明上面那个空是真收敛，不是坏判据。
    const fixture = {
      keys: ["fixture.key", "dialogs.input.placeholder"],
      values: {
        en: { "fixture.key": "Loading…", "dialogs.input.placeholder": "Type here..." },
        ja: { "fixture.key": "読み込み中...", "dialogs.input.placeholder": "入力..." },
        de: { "fixture.key": "Wird geladen…", "dialogs.input.placeholder": "Eingabe..." },
      },
    };
    expect(findEllipsisFormDivergences(fixture)).toEqual(["fixture.key"]);
    // 形态一致的键不报（en/ja/de 同为 `...`），说明上面那条不是「恒返回全部」
    expect(ellipsisForm(r.packs.values.en["dialogs.input.placeholder"])).toBe("...");
    expect(ellipsisForm(r.packs.values.ja["dialogs.input.placeholder"])).toBe("...");
    expect(ellipsisForm(r.packs.values.de["dialogs.input.placeholder"])).toBe("...");
  });
});
