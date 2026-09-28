import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
  deriveBlindSet,
  deriveEnglishAllowlist,
  extractFrontendPending,
  isAdjudicated,
  isBlind,
  isEnglishCandidate,
  listFrontendFiles,
  measure,
  normalizeAdjudication,
  readAdjudication,
  rekey,
  scanLiterals,
  unadjudicated,
} from "./find-english-blind.mjs";

/**
 * 每个夹具断言都配一条**阳性对照**。
 *
 * 阳性对照验证的是【工具】，不是【覆盖】：证明「扫描器在能命中时确实命中」。
 * 少了它，一个「永远返回空」的坏扫描器与一个「真的干净」的输入在输出上无法区分——
 * 本仓已经在这上面翻过两次车。
 */

const ROOT = process.cwd();

/** 夹具根目录：`scripts/check-i18n.mjs` + `src/` 的最小骨架。 */
const tmpRoots = [];
function fixtureRoot(files) {
  const root = mkdtempSync(join(tmpdir(), "eng-blind-"));
  tmpRoots.push(root);
  mkdirSync(join(root, "scripts"), { recursive: true });
  for (const [rel, body] of Object.entries(files)) {
    const full = join(root, rel);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, body);
  }
  return root;
}
afterAll(() => {
  for (const r of tmpRoots) rmSync(r, { recursive: true, force: true });
});

/** 闸门源码的骨架：白名单数组 + 一行占位。 */
function gateWith(arrayBody) {
  return `const OTHER = 1;\nconst FRONTEND_PENDING = [\n${arrayBody}\n];\n`;
}

describe("extractFrontendPending", () => {
  // 真实闸门源码的白名单在计划 7 清空（最后一条 usePricingCatalog.ts 随厂商身份拆分移除）。
  // 断言空表而不是删掉本用例：它钉住的是「没有豁免」这个终态——往里加回一条就会红。
  // 提取器在空表上仍然工作（数组收尾符照旧匹配），下面几条夹具用例证明它不是「只会返回空」。
  it("真实闸门源码的白名单已清空", () => {
    const src = readFileSync(join(ROOT, "scripts/check-i18n.mjs"), "utf8");
    expect(extractFrontendPending(src)).toEqual([]);
  });

  // 阳性对照：证明提取器在有多条时确实逐条取出，而不是只会撞上那一条真值。
  it("多条时逐条提取", () => {
    const src = gateWith('  "src/a.ts",\n  "src/b.vue",');
    expect(extractFrontendPending(src)).toEqual(["src/a.ts", "src/b.vue"]);
  });

  // 提取前先剥注释：注释里出现带引号的路径时，朴素正则会把**注释里的字符串**
  // 当成白名单条目，那个文件随即被从盲文件集合里静默剔除。
  it("注释里的带引号路径不算白名单条目", () => {
    const src = gateWith('  // 曾经遮住 "src/composables/legacy.ts" 这条目录\n  "src/a.ts",');
    expect(extractFrontendPending(src)).toEqual(["src/a.ts"]);
  });

  // 提取不到就抛：静默返回空表会让「整个前端都不在白名单里」，闸门与测量双双失真。
  it("找不到数组时抛错，不静默返回空表", () => {
    expect(() => extractFrontendPending("const X = [];\n")).toThrow(/FRONTEND_PENDING/);
  });
});

describe("listFrontendFiles / isBlind / deriveBlindSet", () => {
  const files = {
    "scripts/check-i18n.mjs": gateWith('  "src/d.ts",'),
    "src/a.vue": "<template><p>Hello</p></template>\n",
    "src/b.vue": "<template><p>你好</p></template>\n",
    "src/c.ts": 'export const greeting = "你好";\n',
    "src/d.ts": 'export const pending = "你好";\n',
    "src/e.test.ts": 'export const skipped = "你好";\n',
  };

  it("只收 .ts/.vue 且排除测试文件", () => {
    const root = fixtureRoot(files);
    expect(listFrontendFiles(root).map((f) => f.slice(root.length + 1))).toEqual([
      "src/a.vue",
      "src/b.vue",
      "src/c.ts",
      "src/d.ts",
    ]);
  });

  it("双零才算盲文件；白名单里的文件两边都不算", () => {
    const root = fixtureRoot(files);
    const set = deriveBlindSet(root);
    expect(set.total).toBe(4);
    expect(set.pending).toEqual(["src/d.ts"]);
    expect(set.blind).toEqual(["src/a.vue"]);
    expect(set.nonBlind).toEqual(["src/b.vue", "src/c.ts"]);
    // 闭合：非白名单文件 = 盲 + 非盲
    expect(set.blind.length + set.nonBlind.length).toBe(set.total - set.pending.length);
  });

  // 阳性对照：两个扫描器各自**能**命中——证明上面那两个「非盲」不是靠常量蒙对的。
  it("阳性对照：模板正文的中文由 findVueTemplateCjkText 抓、脚本区的中文由 findCjkLiterals 抓", () => {
    const b = readFileSync(join(fixtureRoot(files), "src/b.vue"), "utf8");
    const c = readFileSync(join(fixtureRoot(files), "src/c.ts"), "utf8");
    expect(isBlind("src/b.vue", b, [])).toBe(false);
    expect(isBlind("src/c.ts", c, [])).toBe(false);
    // 对照：同一份内容去掉中文后就是盲文件
    expect(isBlind("src/b.vue", "<template><p>Hello</p></template>\n", [])).toBe(true);
  });
});

describe("scanLiterals：五类位置", () => {
  const scan = (rel, src) => scanLiterals(rel, src, { unparsed: [], unhandled: new Set() });
  const kinds = (rel, src) => scan(rel, src).map((i) => `${i.kind}\t${i.text}`);

  it("① 模板文本节点（type === 2）", () => {
    expect(kinds("a.vue", "<template><p>Save changes</p></template>")).toContain("tpl-text\tSave changes");
  });

  it("② 静态属性（type === 6，仅 title/aria-label/placeholder/alt）", () => {
    const src = '<template><img alt="Logo" class="hero" title="Tooltip" /></template>';
    expect(kinds("a.vue", src)).toContain("tpl-attr:alt\tLogo");
    expect(kinds("a.vue", src)).toContain("tpl-attr:title\tTooltip");
    // 阳性对照的反面：不在名单里的静态属性不是文案位，不报
    expect(kinds("a.vue", src).some((k) => k.includes("class"))).toBe(false);
  });

  it("③ 绑定值里的模板字面量（type === 7）", () => {
    const src = "<template><div :title=\"`a ${x} b`\" /></template>";
    expect(kinds("a.vue", src)).toContain("tpl-bind\ta … b");
  });

  it("④ 脚本区（TS AST）", () => {
    expect(kinds("a.ts", 'const label = "Save changes";')).toContain("script\tSave changes");
    expect(kinds("a.vue", '<template><p>x</p></template>\n<script setup>\nconst label = "Save changes";\n</script>')).toContain(
      "script\tSave changes"
    );
  });

  it("★ ⑤ 补漏层：模板插值与指令表达式里的引号字面量（脚本块扫描覆盖不到）", () => {
    const src = "<template><div :class=\"'foo-bar'\">{{ 'Hello there' }}</div></template>";
    const hits = kinds("a.vue", src);
    expect(hits).toContain("tpl-expr\tfoo-bar");
    expect(hits).toContain("tpl-expr\tHello there");
    // 阳性对照：同一串写在脚本区里，走的是另一条路径（`script`），证明两层互不替代
    expect(kinds("a.ts", "const c = 'Hello there';")).toContain("script\tHello there");
  });

  it("行号指向源文件里的真实位置", () => {
    const src = "<template>\n  <p>One</p>\n  <p>Two</p>\n</template>";
    const hit = scan("a.vue", src).find((i) => i.text === "Two");
    expect(hit.line).toBe(3);
  });

  // 文本节点常以「换行 + 缩进」开头，span 从上一行起算——报 span 起始行会让行号
  // 在源文件里指不到那串字。实测曾因此把两个**本来正确**的计划行号「订正」错。
  it("文本节点以换行+缩进开头时，报文字所在行而不是 span 起始行", () => {
    const src = "<template>\n  <span>\n    SYSTEM PROMPT\n  </span>\n</template>";
    const hit = scan("a.vue", src).find((i) => i.text.includes("SYSTEM PROMPT"));
    expect(hit.line).toBe(3);
    // 阳性对照：同一语义的单行写法仍报它自己那一行，说明这不是「一律 +1」
    const inline = scan("a.vue", "<template>\n  <span>SYSTEM PROMPT</span>\n</template>").find((i) =>
      i.text.includes("SYSTEM PROMPT")
    );
    expect(inline.line).toBe(2);
  });
});

describe("scanLiterals：刻意排除的位置", () => {
  const scan = (rel, src) => scanLiterals(rel, src, { unparsed: [], unhandled: new Set() });

  // 日志不进语言包（spec 明写），闸门也在规则 1 之前剥掉 console.*
  it("console.* 的实参不扫；同一串在别处照扫（阳性对照）", () => {
    expect(scan("a.ts", 'console.warn("Failed to save");')).toEqual([]);
    expect(scan("a.ts", 'const m = "Failed to save";').map((i) => i.text)).toEqual(["Failed to save"]);
  });

  it("import/require 的模块说明符不扫；属性名不扫", () => {
    expect(scan("a.ts", 'import x from "./foo";\nconst y = require("./bar");')).toEqual([]);
    expect(scan("a.ts", 'const o = { "a-b": 1 };')).toEqual([]);
    // 阳性对照：同样的字符串当值用时会报
    expect(scan("a.ts", 'const p = "./foo";').map((i) => i.text)).toEqual(["./foo"]);
  });
});

describe("isEnglishCandidate", () => {
  it("模板正文里含字母即候选（含单字母与带前导空格的）", () => {
    expect(isEnglishCandidate(" files", "tpl-text")).toBe(true);
    expect(isEnglishCandidate(" W ", "tpl-text")).toBe(true);
    expect(isEnglishCandidate("Markdown (.md)", "tpl-text")).toBe(true);
  });

  it("脚本区的候选要过形状判据；前导空格参与判定", () => {
    expect(isEnglishCandidate("AI Assistant", "script")).toBe(true);
    expect(isEnglishCandidate("Token", "script")).toBe(true);
    expect(isEnglishCandidate("stop: ", "script")).toBe(true);
    // `" [Error]"`（原在 `reqMessages.ts:245`，现为语言包 `common.reqMessages.errorTag` 的取值）：
    // trim 掉再判会把这条真实候选整条丢掉
    expect(isEnglishCandidate(" [Error]", "script")).toBe(true);
  });

  it("代码形态不是候选", () => {
    for (const s of [
      "../../utils/invokeApp",
      "app.dialog.error",
      "is-startup-active",
      "rgba(0, 0, 0, 0.4)",
      "calc(9px + (100% - 18px))",
      "ANTHROPIC_AUTH_TOKEN",
      "M21.751 22.607c1.34 1.005",
      "update:cliId",
    ]) {
      expect(isEnglishCandidate(s, "script"), s).toBe(false);
    }
    // 阳性对照：把同一批串的「代码形态」去掉后确实变成候选——
    // 证明上面那批 false 是判据判出来的，不是函数恒返回 false。
    expect(isEnglishCandidate("Save changes", "script")).toBe(true);
  });
});

describe("裁决集定位判据是 (文件, 文本)", () => {
  const VUE = "<template>\n  <span>Save changes</span>\n</template>\n";
  // 裁决集里【没有任何行号】：键只有文件与文本。行号漂移因此不可能影响它。
  const ADJ = JSON.stringify({
    "src/A.vue": { "Save changes": { verdict: "keep", reason: "夹具" } },
  });
  // 夹具根目录也要有闸门源码：`measure` 从它那里提取前端白名单（见 `deriveBlindSet`）。
  const FIXTURE_GATE = { "scripts/check-i18n.mjs": gateWith("") };

  it("行号从未出现在裁决集里，候选仍不算未裁决", () => {
    const root = fixtureRoot({ ...FIXTURE_GATE, "src/A.vue": VUE, "scripts/i18n-english-adjudication.json": ADJ });
    expect(unadjudicated(root)).toEqual([]);
  });

  // 阳性对照：证明上面的空数组来自「认得出」，不是来自「扫描器恒返回空」。
  it("阳性对照：裁决集里没有的文本仍算未裁决", () => {
    const root = fixtureRoot({
      ...FIXTURE_GATE,
      "src/A.vue": VUE.replace("Save changes", "Delete forever"),
      "scripts/i18n-english-adjudication.json": ADJ,
    });
    const out = unadjudicated(root);
    expect(out.length).toBe(1);
    expect(out[0]).toContain("Delete forever");
  });

  // `#N` 兜底：同一文件里同一文本有多处、判断不同时，第 2 处起带后缀，故查找要按基名比。
  it("`#N` 后缀按去掉后缀的基名比对", () => {
    const adj = { "src/A.vue": { "Save changes#2": { verdict: "keep", reason: "夹具" } } };
    expect(isAdjudicated(adj, "src/A.vue", "Save changes")).toBe(true);
    // 阳性对照：后缀不是通配符——基名不同的文本、别的文件，都不认
    expect(isAdjudicated(adj, "src/A.vue", "Save change")).toBe(false);
    expect(isAdjudicated(adj, "src/B.vue", "Save changes")).toBe(false);
  });
});

/**
 * 规则 8 的允许清单是**裁决集的机器可读投影**——这条设计成立的前提是「提交的清单 == 当场派生的
 * 结果」。此前没有任何检查守着这个等式：手改清单、或加了 keep 忘了 `--rekey`，都不会红，
 * 而闸门读的正是清单，于是它照常绿（`check-i18n.mjs` 里那句「它由裁决集派生」是**无人校验的声称**）。
 *
 * 与规则 9 的同类断言（`gen-drift-allowlist.test.mjs`）同构：等值断言必须配**阳性对照**——
 * 只断言「两边相等」时，一个恒返回 `{}` 的派生与一份真的空清单在输出上无法区分。
 */
describe("规则 8 允许清单是裁决集的投影", () => {
  const ALLOWLIST = join(ROOT, "scripts/i18n-english-allowlist.json");

  it("提交的 scripts/i18n-english-allowlist.json 等于在内存里派生的结果", () => {
    const adj = readAdjudication(ROOT);
    const { allowlist, emitted, skipped } = deriveEnglishAllowlist(ROOT, adj);
    // 阳性对照一：派生确实读到了裁决集与 src/ 下的 .vue——否则「两边都是空」也会相等
    expect(emitted.length).toBeGreaterThan(0);
    expect(skipped.length).toBeGreaterThan(0);
    expect(Object.keys(allowlist).length).toBeGreaterThan(0);
    // 阳性对照二：清单的每个键都能在裁决集里找到一条 keep（口径是「keep 且规则 8 真会命中」）——
    // 手写进清单的条目会在这里露出来
    const keepTexts = new Set(
      normalizeAdjudication(adj)
        .filter((e) => e.verdict === "keep")
        .map((e) => e.text.trim())
    );
    expect(Object.keys(allowlist).filter((t) => !keepTexts.has(t))).toEqual([]);

    expect(JSON.parse(readFileSync(ALLOWLIST, "utf8"))).toEqual(allowlist);
  });

  it("阳性对照：扰动裁决集里一条 keep 的 reason，等值断言即失效", () => {
    const committed = JSON.parse(readFileSync(ALLOWLIST, "utf8"));
    const targetText = Object.keys(committed)[0];
    const adj = readAdjudication(ROOT);
    const entries = normalizeAdjudication(adj).filter((e) => e.verdict === "keep" && e.text.trim() === targetText);
    expect(entries, `裁决集里找不到产出 ${JSON.stringify(targetText)} 的 keep`).not.toEqual([]);
    // 就地改内存里的裁决集（不落盘）：产出该文本的那一条（可能不止一条）reason 全改掉
    const raw = adj[entries[0].rel];
    for (const k of Object.keys(raw)) {
      if (k.replace(/#\d+$/, "").trim() === targetText) raw[k] = { ...raw[k], reason: `${raw[k].reason}（夹具扰动）` };
    }

    const { allowlist } = deriveEnglishAllowlist(ROOT, adj);
    expect(allowlist).not.toEqual(committed);
    // 差异**只**落在被扰动的那条上：否则这条对照可能在测别的东西
    expect(Object.keys(committed).filter((k) => committed[k] !== allowlist[k])).toEqual([targetText]);
  });
});

/**
 * `rekey` 掌管着**人的判断**（今天 250 条），它的每个分支都只被手工跑过。这里按分支各钉一条：
 * 冲突（停下且不写）、孤儿（保留而非删除）、合并与 `keptApart`（`文本` / `文本#2` 的拆写）。
 */
describe("rekey：冲突 / 孤儿 / 合并 / keptApart", () => {
  const VUE = "<template>\n  <span>Save changes</span>\n</template>\n";
  const FIXTURE_GATE = { "scripts/check-i18n.mjs": gateWith("") };
  /** 夹具：一个盲的 `src/A.vue` + 一份只有它的裁决集。 */
  const fixture = (fileBody) =>
    fixtureRoot({
      ...FIXTURE_GATE,
      "src/A.vue": VUE,
      "scripts/i18n-english-adjudication.json": `${JSON.stringify({ "src/A.vue": fileBody }, null, 2)}\n`,
    });
  const readWritten = (root) =>
    JSON.parse(readFileSync(join(root, "scripts/i18n-english-adjudication.json"), "utf8"));

  it("verdict 冲突：报出、停下，一个文件都不写", () => {
    const root = fixture({
      "Save changes": { verdict: "keep", reason: "甲" },
      "Save changes#2": { verdict: "migrate", reason: "乙" },
    });
    const adjPath = join(root, "scripts/i18n-english-adjudication.json");
    const before = readFileSync(adjPath, "utf8");
    const res = rekey(root);
    expect(res.conflicts.map((c) => [c.rel, c.text])).toEqual([["src/A.vue", "Save changes"]]);
    expect(res.written).toBe(false);
    expect(readFileSync(adjPath, "utf8")).toBe(before);
    // 投影也不写：半写会让两个文件停在互相矛盾的状态
    expect(existsSync(join(root, "scripts/i18n-english-allowlist.json"))).toBe(false);
  });

  it("孤儿：文本不在候选里的条目不删，原样留在 _orphans 下", () => {
    const root = fixture({
      "Save changes": { verdict: "keep", reason: "甲" },
      "Never a candidate": { verdict: "keep", reason: "乙" },
    });
    const res = rekey(root);
    expect(Object.keys(res.orphans)).toEqual(["src/A.vue:Never a candidate"]);
    expect(res.orphans["src/A.vue:Never a candidate"]).toEqual({
      text: "Never a candidate",
      verdict: "keep",
      reason: "乙",
    });
    const written = readWritten(root);
    expect(written._orphans["src/A.vue:Never a candidate"].reason).toBe("乙");
    // 阳性对照：活条目照常搬过去，且没有跑进 _orphans
    expect(written["src/A.vue"]).toEqual({ "Save changes": { verdict: "keep", reason: "甲" } });
  });

  it("同一 (文件, 文本) 判断逐字相同 → 合并成一个键", () => {
    const root = fixture({
      "Save changes": { verdict: "keep", reason: "甲" },
      "Save changes#2": { verdict: "keep", reason: "甲" },
    });
    const res = rekey(root);
    expect(res.merged).toEqual([{ rel: "src/A.vue", text: "Save changes", n: 2 }]);
    expect(res.keptApart).toEqual([]);
    expect(readWritten(root)["src/A.vue"]).toEqual({ "Save changes": { verdict: "keep", reason: "甲" } });
    // 临时文件不留在树里（两个数据文件是「先写 .tmp、再 rename」落盘的）
    expect(readdirSync(join(root, "scripts")).filter((n) => n.endsWith(".tmp"))).toEqual([]);
  });

  it("同一 (文件, 文本) 理由不同 → 按出现序拆成 文本 / 文本#2", () => {
    const root = fixture({
      "Save changes": { verdict: "keep", reason: "甲" },
      "Save changes#2": { verdict: "keep", reason: "乙" },
    });
    const res = rekey(root);
    expect(res.keptApart).toEqual([{ rel: "src/A.vue", text: "Save changes", n: 2 }]);
    expect(res.merged).toEqual([]);
    expect(readWritten(root)["src/A.vue"]).toEqual({
      "Save changes": { verdict: "keep", reason: "甲" },
      "Save changes#2": { verdict: "keep", reason: "乙" },
    });
    // 清单每个值只能有一个理由：取先出现的那条，且把「还有别的理由」报出来
    expect(res.allow.allowlist).toEqual({ "Save changes": "甲" });
    expect(res.allow.reasonedTwice).toEqual([{ text: "Save changes", reasons: ["甲", "乙"] }]);
  });
});

/**
 * 本用例测的是**仓库的当前状态**，不是脚本机制——本文件其余用例都在夹具上跑。
 *
 * **为什么必须有它**：`find-english-blind.mjs` 不在 CI、也不在任何 npm script 里，
 * 它的机制测试从不检查仓库现状。于是「脚本区新写一条英文字面量」这个向量
 * （规则 8 刻意不覆盖，理由见闸门里 findUnlistedEnglishTexts 的说明）
 * **唯一的信号只是闸门那行 note——而 note 不会失败**。
 * 本用例把那行提示变成**一个会失败的检查**：每条候选要么已迁移（不再出现在候选里），
 * 要么在裁决集里，**不允许第三类**。这正是本计划开头那条验收线的机器化。
 *
 * 判据由 `unadjudicated` 提供，与 `--rekey` 共用同一份实现——本用例不自己再写一遍。
 */
describe("仓库现状：每条候选都在裁决集里", () => {
  it("dump 的候选与裁决集的差集为空", () => {
    const r = measure(ROOT);
    const rest = unadjudicated(ROOT);
    // 阳性对照：本次测量确实取到了候选——否则「差集为空」与「扫描器恒返回空」
    // 在输出上无法区分，而这正是本文件开头那条纪律。
    expect(r.candidateCount).toBeGreaterThan(0);
    expect(
      rest,
      `以下候选不在 scripts/i18n-english-adjudication.json 里（新候选须先逐条裁定再提交）：\n${rest.join("\n")}`,
    ).toEqual([]);
  });
});
