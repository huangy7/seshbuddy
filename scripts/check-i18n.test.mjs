import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
  findCjkLiterals,
  findCjkRustLiterals,
  findCjkRustProductionLiterals,
  compareNamespaces,
  findVueTemplateCjkText,
  findHtmlCjkText,
  findUnsupportedHtmlLang,
  findTranslationKeyRefs,
  findDataKeyRefs,
  resolveMessageKey,
  findNamespaceShapeMismatches,
  findVueScriptBlocks,
  findModuleLoadT,
  findUnlistedEnglishTexts,
  findUnlistedDrift,
  findCodedCodeLiterals,
  findCodePackMismatches,
  findPlaceholderMismatches,
  findCodedWithKeyMismatches,
  stripJsComments,
  findStripMisalignment,
  stripRustComments,
} from "./check-i18n.mjs";

// 剥注释是规则 1/2/5 共用的地基：字符串字面量里的 `/*` 一旦被当成注释起始，
// 其后直到下一个 `*/` 的真实代码全部不可见，闸门却照常报「通过」。
// 这是共享基础设施里的静默跳过，用例必须钉住字符串与注释的边界。
// 夹具末尾都留一个 `*/`：没有它，旧实现找不到注释终点反而不吞，用例就测不出这个 bug。
describe("stripJsComments", () => {
  it("字符串里的 /* 不开启块注释，其后的代码不被吞掉", () => {
    expect(stripJsComments('const a = "/* not a comment"; const b = 1;\n/* 真注释 */')).toContain(
      "const b = 1;"
    );
  });

  it("单引号与模板字面量里的 /* 同样不算注释起始", () => {
    expect(stripJsComments("const a = '/*'; const b = 1;\n/* 真注释 */")).toContain("const b = 1;");
    expect(stripJsComments("const a = `/*`; const b = 1;\n/* 真注释 */")).toContain("const b = 1;");
  });

  it("字面量里的转义引号不提前结束字符串", () => {
    expect(stripJsComments('const a = "\\"/*"; const b = 1;\n/* 真注释 */')).toContain("const b = 1;");
  });

  // 字符串里的 // 旧实现本就吞不掉（其行注释正则遇到引号即止），这条是防回归的守卫。
  it("字符串里的 // 不开启行注释", () => {
    expect(stripJsComments('const url = "http://example.com"; const b = 1;')).toContain("const b = 1;");
  });

  // 正则字面量里的转义斜杠（`\/`）后面紧跟的 `/` 不是注释起始。
  // 少了这条守卫，`/^https?:\/\//i` 会被读成 `//` 行注释，把该行其后的真实代码整段剥掉；
  // 被剥掉的那段今天既无中文也不含 t()，规则 1/5 都不会响——回归只能靠这条用例挡住。
  it("正则字面量里的转义斜杠不开启行注释，行尾真实代码不被剥掉", () => {
    expect(stripJsComments("const re = /^https?:\\/\\//i; const b = 1;")).toContain("const b = 1;");
    expect(stripJsComments('const a = x.replace(/^models\\//, ""); const b = 1;')).toContain("const b = 1;");
  });

  // `\` 分支的可达性判据：`\` 之后紧跟的 `/*` **不**开启块注释（转义序列整体跳过，其中的 / 是
  // 内容而非注释起始），故 `x` 必须原样留在输出里。少了这一支，`/*x*/` 会被当成注释整段涂白。
  // 这条钉的是「分支真的被走到」——.vue 的 Windows 路径夹具只断言 exit 0，分不开走到与没走到。
  it("`\\` 之后的 `/*` 不开启块注释（钉住 `\\` 分支被走到）", () => {
    expect(stripJsComments("const a = \\/*x*/ const b = 1;")).toContain("x");
  });

  it("真正的块注释与行注释照旧剥掉", () => {
    expect(stripJsComments("/* 注释 */ const a = 1; // 行注释")).not.toContain("注释");
  });

  // 规则 5 要报行号：等长空白必须保留换行，且长度与原文一致。
  it("keepOffsets 下长度与行结构不变，注释字符换成空白", () => {
    const src = 'const a = "/* not a comment";\n/* 块\n注释 */\n// 行\nconst b = 1;\n';
    const stripped = stripJsComments(src, true);
    expect(stripped.length).toBe(src.length);
    expect(stripped.split("\n").length).toBe(src.split("\n").length);
    expect(stripped).toContain('const a = "/* not a comment";');
    expect(stripped).toContain("const b = 1;");
    expect(stripped).not.toContain("块");
  });

  // 正则字面量里的引号曾被当成字符串起始，引号配对从此错位，**该文件其余部分再也不会被扫描**。
  // src/utils/json.ts 第 9 行的 `.replace(/"/g, "&quot;")`（行号在 `987b91a0` 上量得）就是本仓的真实写法。
  it("正则字面量里的引号不错位，其后的真实代码照常保留", () => {
    expect(stripJsComments('const re = /"/g; const b = 1;')).toContain("const b = 1;");
    expect(stripJsComments("const a = s.replace(/&/g, \"&amp;\").replace(/\"/g, \"&quot;\"); const b = 1;")).toContain(
      "const b = 1;"
    );
  });

  // 正则体整体置空：里面的引号与花括号是数据，留着会打乱下游的引号配对与深度记账。
  it("正则体整体置空，其中的花括号不参与深度记账", () => {
    expect(stripJsComments("const re = /{/; const b = 1;")).not.toContain("/{/");
    expect(stripJsComments("const re = /{/; const b = 1;")).toContain("const b = 1;");
  });

  // `</div>` 遍地都是，而 `a < /re/` 没人写。把 `</` 认成正则起始会一路吞到同一行下一个 `/`，
  // 标签之间的真实代码整段消失——实测 UsageDashboard.vue 一行里的 t() 引用就是这样丢的。
  it("HTML 闭合标签不被当成正则起始，标签之间的代码不被吞掉", () => {
    expect(stripJsComments('<div></div>{{ t("x") }}</div>')).toContain('t("x")');
    expect(stripJsComments('<tr><th>{{ t("a.b") }}</th><th>{{ t("c.d") }}</th></tr>')).toContain('t("c.d")');
  });

  // `+` / `-` 是正则前置字符，于是 `i++ / 2; // 注释` 里的 ` / 2; /` 被当成正则体吞掉，
  // 连同 `;` 与 `//` 起始一起：注释正文留在剥好的正文里，规则 1 会把注释里的中文报成硬编码。
  // 成对的 `++` / `--` 之后 `/` 必然是除号（后缀自增的表达式已经完整）。
  it("自增自减之后的除号不被当成正则起始，注释照常剥掉", () => {
    expect(stripJsComments("let i = 0;\ni++ / 2; // 注释\nconst b = 1;")).not.toContain("注释");
    expect(stripJsComments("let i = 0;\ni-- / 2; // 注释\nconst b = 1;")).not.toContain("注释");
  });

  // 单个 `+` / `-` 之后照旧按正则处理：`a + +/{/` 里的两个 `+` 隔着空白，是二元加号接一元加号
  // （合法 JS），正则体必须整体置空。整类移出集合、或把成对判据放宽到「隔着空白也算」，
  // 都会让那个 `{` 混进深度记账，在一份合法文件上报花括号不平衡。
  it("一元加号之后的正则照旧整体置空", () => {
    expect(stripJsComments("const ok = a + /{/.test(s);\nconst b = 1;")).not.toContain("/{/");
    expect(stripJsComments("const ok = a + +/{/.test(s);\nconst b = 1;")).not.toContain("/{/");
  });

  // `.vue` 是按整份文件剥的，模板正文与 HTML 注释里的撇号（英文文案里遍地都是）曾被当成
  // 引号起始：引号配对错位后一路吞到文件末尾，其后真实代码整段不可见，闸门在一份完全合法的
  // 文件上 exit 1。单/双引号字符串不能跨行，故同行找不到收尾引号的引号不是字符串起始。
  // 断言落在「其后那行注释被剥掉」上：字面量是原样保留的，只看正文还在不在测不出这个 bug。
  it("模板正文与 HTML 注释里的撇号不开字符串，其后代码照常可见", () => {
    expect(stripJsComments("<template><p>Don't save</p></template>\nconst b = 1; // 注释")).not.toContain("注释");
    expect(stripJsComments("<!-- don't render this -->\nconst b = 1; // 注释")).not.toContain("注释");
    // 同行有第二个撇号时同样不吞掉其后的真实代码。
    expect(stripJsComments("<p>You're right, it's fine</p>\nconst b = 1; // 注释")).not.toContain("注释");
  });

  // 反引号不能跟着这样处理：模板字面量合法跨行，同行没有收尾反引号说明不了什么。
  // 断言落在「字面量里的 // 是内容」上：退化成按行处理时它会被当成注释剥掉。
  it("跨行的模板字面量照旧整体保留，其中的 // 是内容", () => {
    const src = "const s = `a\n// 内容\nb`;\nconst c = 1;";
    expect(stripJsComments(src)).toContain("// 内容");
    expect(stripJsComments(src)).toContain("const c = 1;");
  });
});

// 剥注释一旦错位，错位点之后的真实代码对规则 1/5 全部不可见，而闸门照常报「通过」。
// 自检把这种静默失明变成响亮的失败：判据不指向任何单一原因，只看后果。
describe("findStripMisalignment", () => {
  it("错位迹象都没有时返回 null", () => {
    expect(findStripMisalignment('const a = 1;\nfunction f() { return { b: 2 }; }\n')).toBeNull();
    expect(findStripMisalignment('const re = /"/g;\nconst s = "保存";\n')).toBeNull();
  });

  // 模板字面量合法跨行，找不到收尾反引号时它一路吞到文件末尾：其后所有代码都不可见。
  // `.vue` 是按整份文件剥的，故脚本块里的一处未闭合就够触发。
  it("模板字面量未闭合时报出", () => {
    expect(findStripMisalignment("const s = `abc\n")).toBe("扫描结束时仍在字符串字面量里");
    expect(findStripMisalignment('<script setup lang="ts">\nconst s = `abc\n</script>\n')).toBe(
      "扫描结束时仍在字符串字面量里"
    );
  });

  // 入参是**脚本区**文本：.ts 是整份文件，.vue 是 <script> 块（见 main() 与本文件末组用例）。
  // 模板区里的撇号与落单花括号是正文而不是代码，由调用方在块外隔离，不在这里判。
  // 单/双引号字符串本就不能跨行，脚本区里出现「同行没有收尾引号」只可能是扫描判错了位置：
  // 旧实现靠「引号一路吞到文件末尾」响亮报错，同行规则接管后必须换成这一位，否则就是把
  // 响亮失败降级成静默漏报（规则 1 看不见其中的中文，规则 5 的引用被掩掉而无人出声）。
  it("脚本区里同行未闭合的单/双引号时报出", () => {
    expect(findStripMisalignment("const a = '你好;\nconst b = 1;\n")).toBe("单/双引号同行没有收尾引号（共 1 处）");
    // 正则被误判成除号后，其中的引号与同一行后文的引号配了对，段边界因此错位——
    // 花括号与模板字面量都没事，只有这一位报得出来。
    expect(findStripMisalignment("if (ok) /'/; const a = t(\"common.a\"); const b = 'x';\n")).toBe(
      "单/双引号同行没有收尾引号（共 1 处）"
    );
  });

  it("合法脚本区不误报", () => {
    expect(findStripMisalignment("const a = '保存';\nconst b = \"don't\";\n")).toBeNull();
    expect(findStripMisalignment("const s = 'a\\\n}';\nconst b = 1;\n")).toBeNull();
    // CRLF 续行是**一次**续行：只跳两个字符会把 \r 留在原地，下一轮把 \n 读成换行而误判。
    // 仓库没有 .gitattributes，Windows 检出（core.autocrlf=true）就会是 CRLF。
    expect(findStripMisalignment("const s = 'a\\\r\n}';\nconst b = 1;\n")).toBeNull();
    // 一元加号接正则：两个 + 之间隔着空白，不是自增运算符，正则体必须照旧整体置空，
    // 否则正则体里的 `{` 混进深度记账，在一份合法文件上报花括号不平衡。
    expect(findStripMisalignment("const ok = a + +/{/.test(s);\nconst b = 1;\n")).toBeNull();
  });

  // 判据不是「有没有正则里的引号」，而是「有没有错位的后果」——除号被误认成正则、
  // 任何让引号配错对的东西，后果只要落在这两种签名里就报出来。
  it("代码位置的花括号不平衡时报出", () => {
    expect(findStripMisalignment("const a = 1;\n}\n")).toBe("代码位置的花括号不平衡（净 -1）");
    expect(findStripMisalignment("if (flag) /}/.test(s);\n")).toBe("代码位置的花括号不平衡（净 -1）");
  });

  it("字符串与注释里的花括号不参与记账，不误报", () => {
    expect(findStripMisalignment('const a = "{";\nconst b = "}";\n// }\n')).toBeNull();
  });

  // 正好闭合在文件末尾的字面量，结束下标就是文件长度，与「一路吞到 EOF」只差一位。
  // 判据写成 `>= src.length` 时这份合法文件会被报成未终止——仓库里三个无尾换行的文件
  // 分别以 `>`、`;`、`;` 结尾，只差一次编辑就会踩上，而报错方向是响亮的（干净树上 exit 1）。
  it("正好闭合在文件末尾的字面量不算未终止", () => {
    expect(findStripMisalignment('const a = "x"')).toBeNull();
    expect(findStripMisalignment("const a = 'y'")).toBeNull();
    expect(findStripMisalignment("const a = `z`")).toBeNull();
  });

  // 代码注释里写着「本仓所有真实文件都能剥干净」——这句话由用例钉住。
  // 新写法一旦让某个真实文件错位，这里立刻变红，而不是等闸门在那份文件上静默失明。
  // 范围与闸门一致：.vue 只查 <script> 块，模板区里的撇号与落单花括号是正文不是代码。
  it("本仓所有真实 .ts/.vue 的脚本区都能剥干净", () => {
    const root = join(process.cwd(), "src");
    const files = [];
    const walk = (dir) => {
      for (const name of readdirSync(dir)) {
        const full = join(dir, name);
        if (statSync(full).isDirectory()) walk(full);
        else if (/\.(ts|vue)$/.test(full)) files.push(full);
      }
    };
    walk(root);
    expect(files.length).toBeGreaterThan(100);
    const misaligned = files.filter((f) => {
      const src = readFileSync(f, "utf8");
      const regions = f.endsWith(".vue") ? findVueScriptBlocks(src).map((b) => b.text) : [src];
      return regions.some((region) => findStripMisalignment(region));
    });
    expect(misaligned.map((f) => relative(root, f))).toEqual([]);
  });
});

describe("findCjkLiterals", () => {
  it("命中字符串字面量里的中文", () => {
    expect(findCjkLiterals('const a = "保存";').length).toBe(1);
  });

  it("忽略行注释与块注释", () => {
    expect(findCjkLiterals('// 这是注释\n/* 也是注释 */').length).toBe(0);
  });

  it("忽略纯英文代码", () => {
    expect(findCjkLiterals('const a = "save";').length).toBe(0);
  });

  it("命中模板字符串中的中文", () => {
    expect(findCjkLiterals("const a = `共 ${n} 条`;").length).toBe(1);
  });

  it("忽略 console.* 的日志文案", () => {
    expect(findCjkLiterals('console.warn("读取界面语言失败，使用基准语言", err);')).toEqual([]);
  });

  it("console.* 之外的字符串照旧命中", () => {
    expect(findCjkLiterals('console.warn("save"); const a = "保存";')).toEqual(["保存"]);
  });

  // 只认汉字会漏掉整类硬编码：`join("、")` 里一个汉字都没有，闸门对它一声不响，
  // 而英文界面上它就是 `Claude、Gemini` 这种可见缺陷。中日韩标点（U+3000–U+303F）
  // 与全角形式（U+FF00–U+FFEF）因此一并算中文文本。
  it("命中只有中日韩标点、没有汉字的字面量", () => {
    expect(findCjkLiterals('const a = "、";')).toEqual(["、"]);
    expect(findCjkLiterals("const a = `「${x}」`;")).toEqual(["「${x}」"]);
    expect(findCjkLiterals('const a = "来源：";')).toEqual(["来源："]);
  });

  it("命中全角形式", () => {
    expect(findCjkLiterals('const a = "Ａ";')).toEqual(["Ａ"]);
    expect(findCjkLiterals('const a = "（全部）";')).toEqual(["（全部）"]);
  });

  it("半角标点与纯英文照旧不报", () => {
    expect(findCjkLiterals('const a = "save, b: c";')).toEqual([]);
  });

  // JS 的字符串同样能用行尾 `\` 续行（LineContinuation），与 Rust 侧是同一道边界：
  // 取串正则里的 `\\.` 中 `.` 不匹配换行，于是那个续行串自身配不上对，引擎从它的**收尾引号**
  // 重新起配——收尾引号被当成了某个串的开引号，其后所有中文串随引号配对一起错位、一条都取不到。
  //
  // 可达条件（与 Rust 侧不同，这里要写清楚）：`'`/`"` 分支的内容类 `[^"\\\n]` **排除换行**，
  // 故只有当续行串的收尾引号与下一个串的开引号**在同一行**时才会错位（隔行时那个换行
  // 本身就让配对失败，配不上去）。Rust 侧的内容类不排除换行，所以那边没有这个限制。
  // 实测今天 src/ 下没有续行串（`command grep -rn '\\$' src --include='*.ts' --include='*.vue'`
  // 零命中，同一 pattern class 在 dist/*.js 上有命中作正对照），故此刻不产生任何误判；
  // 但闸门是后面每个任务验收的地基——前端白名单一旦重开，那份白名单就会按错数字重建。
  it("行尾续行的字符串不让其后整个文件的引号配对错位", () => {
    const withContinuation = [
      'const sql = "SELECT a \\',
      '  FROM t"; const label = "保存";',
    ].join("\n");
    expect(findCjkLiterals(withContinuation)).toEqual(["保存"]);
    // 对照组：把那个续行（`\` + 换行）整个抹掉，字符串回到单行——同一条中文串照旧命中，
    // 证明它确实落在扫描范围内，上面的命中不是靠别的路径凑出来的。
    const control = withContinuation.replace('"SELECT a \\\n  FROM t"', '"SELECT a FROM t"');
    expect(control).not.toBe(withContinuation);
    expect(findCjkLiterals(control)).toEqual(["保存"]);
  });
});

// stripRustComments 是 stripJsComments 的另一半：同一道边界，另一种语言。
// 字符串字面量里的块注释起始符一旦被当成注释起点，其后直到下一个结束符的真实代码
// 对规则 2 全部不可见，闸门却照常报「通过」。
describe("stripRustComments", () => {
  it("字符串里的 /* 不开启块注释，其后的代码不被吞掉", () => {
    expect(stripRustComments('let a = "a /* b";\nlet c = "加载中";\n/* 真注释 */\nlet d = 2;')).toContain(
      "加载中"
    );
  });

  it("真正的块注释与行注释照旧剥掉", () => {
    const stripped = stripRustComments('/* 注释 */ let a = 1; // 行注释\nlet b = "值";');
    expect(stripped).not.toContain("注释");
    expect(stripped).toContain("值");
  });

  // 内层注释的结束符不结束外层。按「找第一个结束符」实现时，内层之后的注释正文
  // 会被当成代码，规则 2 就对注释里的中文误报。
  it("嵌套块注释按深度配对，内层之后的注释正文不被当成代码", () => {
    const stripped = stripRustComments('/* 外 /* 内 */ 仍是注释 */ let a = "值";');
    expect(stripped).not.toContain("仍是注释");
    expect(stripped).toContain("值");
  });

  // `'` 在 Rust 里首先是生命周期标记。当成字符串起始会让两次生命周期之间的真实代码
  // 落进「字符串」里，其中的中文对规则 2 不可见。
  it("生命周期标记不开启字符串，其后的字面量照旧可见", () => {
    expect(stripRustComments("fn f<'a>(x: &'a str) -> &'a str { \"中文\" }")).toContain("中文");
  });

  // 原始字符串里的 `\` 不是转义符；内容里还可以有不带转义的 `"`，
  // 原样保留会让引号个数变成奇数，下游按引号配对取字面量的正则从此错位。
  it("原始字符串不提前收尾，且不打乱后续的引号配对", () => {
    expect(stripRustComments('let a = r#"{"k": "v"}"#;\nlet c = "保存";')).toContain("保存");
    expect(stripRustComments('let a = r#"a"b"#;\nlet c = "保存";')).toContain("保存");
  });
});

describe("findCjkRustLiterals", () => {
  it("命中 Rust 字面量里的中文", () => {
    expect(findCjkRustLiterals('const A: &str = "保存";')).toEqual(["保存"]);
  });

  it("忽略纯英文代码", () => {
    expect(findCjkRustLiterals('const A: &str = "save";')).toEqual([]);
  });

  it("忽略日志宏的文案", () => {
    expect(findCjkRustLiterals('tracing::warn!("重建托盘菜单失败: {}", e);')).toEqual([]);
    expect(findCjkRustLiterals('println!("保存 {}", x);')).toEqual([]);
  });

  // 含 `/*` 的字符串会把其后直到下一个结束符的真实代码整段抹掉——规则 2 看不见其中的
  // 硬编码中文，闸门照常报「通过」。
  it("字符串里的 /* 之后的硬编码中文不再被静默跳过", () => {
    const fixture = 'fn a() {\n    let re = "a /* b";\n    let c = "加载中";\n    /* real comment */\n    let d = 2;\n}';
    expect(findCjkRustLiterals(fixture)).toEqual(["加载中"]);
    // 对照组一：去掉那个 `/*`，结果不变——证明中文确实落在扫描范围内，命中不是碰巧。
    expect(findCjkRustLiterals(fixture.replace("/* b", "/ b"))).toEqual(["加载中"]);
    // 对照组二：同一夹具交给被淘汰的正则版剥注释，中文整段消失——证明这条用例
    // 钉的正是「字符串里的注释起始符」这个边界，而不是别的路径凑出来的。
    const naive = fixture.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(findCjkRustLiterals(naive)).toEqual([]);
  });

  // 字符字面量里的 `"`（`matches!(ch, '"' | …)` 这类写法仓库里就有）同样会让引号错位。
  it("含引号的字符字面量不打乱后续的引号配对", () => {
    expect(findCjkRustLiterals(`let x = '"'; let s = "保存";`)).toEqual(["保存"]);
  });

  // Rust 的字符串可以跨行：行尾 `\` 续行，本仓的 SQL 串遍地都是
  // （`"INSERT INTO … \` 换行 `VALUES (…)`）。取串正则里的 `\\.` 中，`.` 不匹配换行，
  // 于是**从这个续行串起引号配对整体错位**，其后所有中文串一条都取不到——
  // 静默失明，闸门照常报「通过」。实测：assistant/conversations.rs 的生产 CJK 因此报 0
  // （真实 3 条，全是 `AppError::business`），assistant/quick_phrases.rs 报 2（真实 3 条）。
  it("行尾续行的字符串不让其后整个文件的引号配对错位", () => {
    const src = [
      "fn f() {",
      '    conn.execute("SELECT a \\',
      '        FROM t")?;',
      '    return Err(AppError::business("对话不存在"));',
      "}",
    ].join("\n");
    expect(findCjkRustLiterals(src)).toEqual(["对话不存在"]);
    // 对照组：去掉那个续行反斜杠，同一条中文串照旧命中——证明它确实落在扫描范围内，
    // 上面的命中不是靠别的路径凑出来的。
    expect(findCjkRustLiterals(src.replace("SELECT a \\", "SELECT a"))).toEqual(["对话不存在"]);
  });
});

// 规则 2 的切片：旧实现 `src.split("#[cfg(test)]")[0]` **假设第一个标记必在文件末尾**。
// 实测不是——三种 span 形态在本仓都存在（`#[cfg(test)]` 共 61 处，实测 mod 54 / fn 6 / use 1），
// 且标记不落在文件末尾的有 8 个文件。三处典型（**以下行号在 `15cac67f` 上量得**——那是这套判据
// 落地时的树，文件其后长过，不要在 HEAD 上按这些数字找行；随附的计数与文件长度都是该树的实测值）：
// parser/claude.rs 第 10 行的 `#[cfg(test)] use` 让其后 2416 行（全文件 2426 行）对规则 2 失明，
// proxy.rs 第 282 行的测试模块到第 348 行结束，生产代码从第 350 行继续，commands/session.rs
// 第 721 行的 `#[cfg(test)] fn` 夹在文件中部。后果不是「少报几处」：这类文件**去掉白名单条目
// 也不会开始受检**——扫描走到标记处就停，闸门照常报「通过」，于是「收紧白名单」只收紧了数字。
//
// 每条夹具都同时断言**生产 CJK 被找到**与**测试 CJK 不被找到**：只断言前者时「把整个文件
// 涂白」也能通过，只断言后者时「按第一个标记一刀切」也能通过，两种错法各需一半才拦得住。
describe("findCjkRustProductionLiterals", () => {
  it("① 测试模块之后还有生产代码时，两段生产 CJK 都检出、测试 CJK 不检出", () => {
    // 测试体里带 JSON 夹具（转义引号 + 落单的半个花括号）。配平必须在**字符串已涂白**的
    // 副本上做：带字符串配平时那个 `{` 让深度一路加下去、span 收不了尾。
    const src = [
      'const A: &str = "生产一";',
      "#[cfg(test)]",
      "mod tests {",
      "    fn t() {",
      '        let bad = "{\\"a\\": 1";',
      '        let s = "测试一";',
      "    }",
      "}",
      'const B: &str = "生产二";',
    ].join("\n");
    expect(findCjkRustProductionLiterals(src)).toEqual(["生产一", "生产二"]);
  });

  // `use` 项没有函数体，遮蔽到分号即止。判据若一律去找下一个 `{`，会把其后直到第一个
  // 函数体的生产代码一起涂掉——夹具后半段的生产 CJK 就是为这一半准备的。
  it("② 文件开头的 `#[cfg(test)] use` 只遮蔽到分号，其后生产代码照常受检", () => {
    const src = [
      "#[cfg(test)]",
      "use crate::session::tool_use_summary;",
      'const A: &str = "生产一";',
      'fn f() { let s = "生产二"; }',
    ].join("\n");
    expect(findCjkRustProductionLiterals(src)).toEqual(["生产一", "生产二"]);
  });

  // 返回类型里的 `[u8; 2]` 含分号：按「第一个分号即项末」实现会在签名中间就收尾，
  // 函数体漏出为生产代码，其中的测试 CJK 被误报。
  it("③ 文件中段的 `#[cfg(test)] fn` 遮蔽整个函数体，前后生产代码都受检", () => {
    const src = [
      'const A: &str = "生产一";',
      "#[cfg(test)]",
      "fn helper() -> [u8; 2] {",
      '    let s = "测试一";',
      "    [0; 2]",
      "}",
      'const B: &str = "生产二";',
    ].join("\n");
    expect(findCjkRustProductionLiterals(src)).toEqual(["生产一", "生产二"]);
  });

  // ④ 是既有正确情形，必须继续通过：修切片不能把本来对的那一半弄坏。
  it("④ 标记在文件末尾时生产代码照常受检（回归守卫）", () => {
    const src = [
      'const A: &str = "生产一";',
      "#[cfg(test)]",
      "mod tests {",
      '    fn t() { let s = "测试一"; }',
      "}",
    ].join("\n");
    expect(findCjkRustProductionLiterals(src)).toEqual(["生产一"]);
  });

  // 本仓真实的两种写法：`pub(crate) fn`（parser/claude.rs）与一个文件里多个测试项
  // （session.rs 3 处、parser/claude.rs 5 处），测试项之间的生产代码必须照常受检。
  it("多个测试项之间的生产代码照常受检，`pub(crate) fn` 形式同样遮蔽", () => {
    const src = [
      "#[cfg(test)]",
      "pub(crate) fn parse_legacy() -> Result<(), String> {",
      '    let s = "测试一";',
      "    Ok(())",
      "}",
      'const A: &str = "生产一";',
      "#[cfg(test)]",
      "mod tests {",
      '    fn t() { let s = "测试二"; }',
      "}",
    ].join("\n");
    expect(findCjkRustProductionLiterals(src)).toEqual(["生产一"]);
  });

  // 配平副本里注释也必须涂白：测试体注释里写一个 `}` 会让深度提前归零、span 在函数体中间
  // 就收尾，其后的测试 CJK 漏成「生产代码」被报出来。
  it("测试体注释里的花括号不参与配平", () => {
    const src = [
      "#[cfg(test)]",
      "mod tests {",
      "    // 说明：这里写的是半个 } 的样子",
      '    fn t() { let s = "测试一"; }',
      "}",
      'const A: &str = "生产一";',
    ].join("\n");
    expect(findCjkRustProductionLiterals(src)).toEqual(["生产一"]);
  });
});

describe("compareNamespaces", () => {
  it("列出目标语言缺失的 key", () => {
    const base = { a: "1", b: "2" };
    const target = { a: "x" };
    expect(compareNamespaces(base, target)).toEqual(["b"]);
  });

  it("key 完全一致时无缺口", () => {
    expect(compareNamespaces({ a: "1" }, { a: "x" })).toEqual([]);
  });

  // 形状不一致时递归会撞上 `in` 对字符串抛 TypeError。抛出会让 main() 在打印已积累的
  // failures 之前中断——同一轮里其它文件的违规全部消失，闸门从「漏报一处」变成
  // 「静默丢弃整轮诊断」。两个方向都不能抛。
  it("一侧是子键集合、另一侧是文本时不抛，报成该键缺失", () => {
    expect(compareNamespaces({ a: { b: "2" } }, { a: "1" })).toEqual(["a"]);
    expect(compareNamespaces({ a: "1" }, { a: { b: "2" } })).toEqual([]);
  });
});

describe("findNamespaceShapeMismatches", () => {
  it("一侧是文本、另一侧是子键集合时报出该键", () => {
    expect(findNamespaceShapeMismatches({ a: "1" }, { a: { b: "2" } })).toEqual(["a"]);
    expect(findNamespaceShapeMismatches({ a: { b: "2" } }, { a: "1" })).toEqual(["a"]);
  });

  it("嵌套处的形状不一致同样报出完整路径", () => {
    expect(findNamespaceShapeMismatches({ x: { y: "1" } }, { x: { y: { z: "2" } } })).toEqual(["x.y"]);
  });

  it("形状一致时无差异（值不同不算差异）", () => {
    expect(findNamespaceShapeMismatches({ a: { b: "1" } }, { a: { b: "2" } })).toEqual([]);
    expect(findNamespaceShapeMismatches({ a: "1" }, { a: "x" })).toEqual([]);
  });

  it("一侧缺键时不报形状（那是缺口，不是形状）", () => {
    expect(findNamespaceShapeMismatches({ a: "1" }, {})).toEqual([]);
  });
});

describe("findTranslationKeyRefs", () => {
  const keys = (src) => findTranslationKeyRefs(src).map((r) => r.key);

  it("取出双引号、单引号与无插值模板字面量的 key", () => {
    expect(keys('t("a.b"); t(\'c.d\'); t(`e.f`);')).toEqual(["a.b", "c.d", "e.f"]);
  });

  it("取出带插值模板字面量的静态前缀", () => {
    expect(findTranslationKeyRefs("t(`a.b.${x}`);")).toEqual([{ key: "a.b", line: 1, dynamic: true }]);
  });

  it("静态前缀截到最后一个点，保留半拉子键名之外的完整路径", () => {
    expect(keys("t(`a.b.c${x}`);")).toEqual(["a.b"]);
  });

  it("前缀里没有点时跳过", () => {
    expect(keys("t(`${ns}.title`); t(`common${x}`);")).toEqual([]);
  });

  it("忽略注释里的 t()", () => {
    expect(keys('// t("a.b")\n/* t("c.d") */\n')).toEqual([]);
  });

  it("块注释按行占位，行号与原文一致", () => {
    expect(findTranslationKeyRefs('/*\n\n*/\nt("a.b");')).toEqual([{ key: "a.b", line: 4, dynamic: false }]);
  });

  it("忽略实参不是字面量的调用", () => {
    expect(keys('t(variable); t("a." + x); t("a.b" + x);')).toEqual([]);
  });

  it("忽略名字里带 t( 的调用与属性访问", () => {
    expect(keys('format("a.b"); obj.t("a.b"); split("a.b"); $t("a.b");')).toEqual([]);
  });

  it("带具名参数的调用照常取出 key", () => {
    expect(keys('t("a.b", { count: 1 });')).toEqual(["a.b"]);
  });

  // 取串正则里的 `\\.` 中 `.` 不匹配换行：key 字面量若写成行尾 `\` 续行（合法 JS），
  // 这条引用整条取不到，规则 5 就不再校验它的存在性——错别字静默通过。
  // 与规则 1/2 同型，但**不级联**：本函数的每条匹配都以 `t(` 起头，一次失败不消耗任何文本
  // （lastIndex 只前进一位），故只会漏掉这一条，其后的 `t(…)` 照旧找到。
  it("行尾续行的 key 字面量照旧取出，不再整条漏掉", () => {
    expect(findTranslationKeyRefs('const a = () => t("common.\\\nkey");')).toEqual([
      { key: "common.\\\nkey", line: 1, dynamic: false },
    ]);
    // 对照组：把续行抹掉（key 回到单行），同一条引用照旧取出——证明这条 `t(…)` 确实在扫描范围内。
    expect(keys('const a = () => t("common.key");')).toEqual(["common.key"]);
  });
});

// changelog.ts 把 i18n key 当数据存放，规则 5 只认 t("字面量")，看不见它们。
describe("findDataKeyRefs", () => {
  const keys = (src) => findDataKeyRefs(src).map((r) => r.key);

  it("取出 *Keys 数组里的字面量", () => {
    expect(keys('const c = [{ changeKeys: ["app.a.b", "app.c.d"] }];')).toEqual(["app.a.b", "app.c.d"]);
  });

  it("报出的行号指向该 key 所在行", () => {
    expect(findDataKeyRefs('const c = {\n  changeKeys: [\n    "app.a.b",\n  ],\n};')).toEqual([
      { key: "app.a.b", line: 3 },
    ]);
  });

  // 放宽成「文件里所有点分字符串」会把 version 这类非 key 数据一并当 key 查，立刻误报。
  it("数组之外的点分字符串不当作 key", () => {
    expect(keys('const c = { version: "0.1.0", date: "2026-09-18" };')).toEqual([]);
  });

  it("忽略注释里的 key 写法", () => {
    expect(keys('// changeKeys: ["app.a.b"]\n/* changeKeys: ["app.c.d"] */')).toEqual([]);
  });

  // 本函数的取串正则是**裸串匹配器**（不像规则 5 那样每条匹配都以 `t(` 起头），
  // 所以 `\\.` 的换行盲会**级联**：错配的那一次把后一个开引号当成自己的收尾引号，
  // 同一个 changeKeys 数组里其后的 key 一条都取不到——错别字静默通过。
  it("行尾续行的 key 不让同一数组里其后的 key 一起漏掉", () => {
    expect(findDataKeyRefs('const c = [{ changeKeys: ["app.a.\\\nb", "app.c.d"] }];').map((r) => r.key)).toEqual([
      "app.a.\\\nb",
      "app.c.d",
    ]);
    // 对照组：把续行抹掉（两个 key 都回到单行），两条照旧取出——证明它们在扫描范围内。
    expect(keys('const c = [{ changeKeys: ["app.a.b", "app.c.d"] }];')).toEqual(["app.a.b", "app.c.d"]);
  });
});

describe("resolveMessageKey", () => {
  const messages = { common: { language: { en: "English" }, empty: {} }, "api-log": { title: "Log" } };

  it("取到文本的是叶子", () => {
    expect(resolveMessageKey(messages, "common.language.en")).toBe("leaf");
  });

  it("取到子键集合的是分支", () => {
    expect(resolveMessageKey(messages, "common.language")).toBe("branch");
  });

  it("不存在的路径报 missing", () => {
    expect(resolveMessageKey(messages, "common.language.en2")).toBe("missing");
  });

  it("空子键集合不当作分支", () => {
    expect(resolveMessageKey(messages, "common.empty")).toBe("missing");
  });

  it("带连字符的命名空间按点分后正常解析", () => {
    expect(resolveMessageKey(messages, "api-log.title")).toBe("leaf");
  });
});

describe("findVueTemplateCjkText", () => {
  it("命中模板里的裸文本节点", () => {
    expect(findVueTemplateCjkText("<template><div>保存</div></template>")).toEqual(["保存"]);
  });

  it("忽略标签内的属性值（含 :title 这类绑定）", () => {
    expect(findVueTemplateCjkText('<template><div :title="保存">save</div></template>')).toEqual([]);
  });

  it("忽略模板注释", () => {
    expect(findVueTemplateCjkText("<template><!-- 保存 --><div></div></template>")).toEqual([]);
  });

  it("忽略 {{ }} 插值，交给字面量扫描", () => {
    expect(findVueTemplateCjkText("<template><div>{{ t('保存') }}</div></template>")).toEqual([]);
  });

  it("不扫 <script> 内容", () => {
    expect(
      findVueTemplateCjkText("<template><div>save</div></template><script>const a = '保存';</script>")
    ).toEqual([]);
  });

  // 与字面量扫描同一字符范围：模板文本节点里的中文标点同样是硬编码文案。
  it("模板文本节点里的中日韩标点同样命中", () => {
    expect(findVueTemplateCjkText("<template><div>、</div></template>")).toEqual(["、"]);
    expect(findVueTemplateCjkText("<template><div>（全部）</div></template>")).toEqual(["（全部）"]);
  });
});

// .vue 的 script 块是模块作用域所在；模板与样式块不是。行号要按块首在文件里的位置折算，
// 否则报出的位置指向文件开头。
describe("findVueScriptBlocks", () => {
  it("取出 script setup 块正文与块首行号", () => {
    const src = '<template><div>save</div></template>\n<script setup lang="ts">\nconst a = 1;\n</script>\n';
    expect(findVueScriptBlocks(src)).toEqual([{ text: "\nconst a = 1;\n", line: 2 }]);
  });

  it("同时存在 script 与 script setup 时两块都取出", () => {
    const src = '<script lang="ts">\nexport default {};\n</script>\n<script setup lang="ts">\nconst a = 1;\n</script>\n';
    expect(findVueScriptBlocks(src).map((b) => b.line)).toEqual([1, 4]);
  });

  it("没有 script 块时返回空", () => {
    expect(findVueScriptBlocks("<template><div>save</div></template>")).toEqual([]);
  });

  // 属性值里可以有 `>`（Vue 的 `generic="T extends Record<string, any>"`）。开标签按第一个 `>`
  // 截断会让块正文以 `">` 开头，那个落单的引号随即把整块置空——静默跳过。
  it("属性值里的 > 不截断块正文", () => {
    const src = '<script setup lang="ts" generic="T extends Record<string, any>">\nconst a = 1;\n</script>\n';
    expect(findVueScriptBlocks(src)).toEqual([{ text: "\nconst a = 1;\n", line: 1 }]);
  });

  // 同一处截断也让块首行号落在属性那一行而不是开标签结束的那一行；多行开标签才看得出来。
  // 行号是「块正文第一个字符所在行」，正文以开标签后的换行起头，故这里是 4 而不是 5。
  it("多行开标签的行号按开标签真正的结束位置折算", () => {
    const src =
      '<script\n  setup\n  generic="T extends Record<string, any>"\n>\nconst a = 1;\n</script>\n';
    expect(findVueScriptBlocks(src)).toEqual([{ text: "\nconst a = 1;\n", line: 4 }]);
  });
});

// 只求值一次的 t()：语言在那一刻被冻结，用户之后切换语言这些文案不跟随。
// 判据是「这条语句里有 t() 调用，且该调用不在任何函数类节点内」——由 AST 判定，
// 不看语句长什么样、不含什么记号。下面逐行对应判据表，
// **不该命中的那几行是这条规则不误报的证据**，与该命中的一样重要。
describe("findModuleLoadT", () => {
  it("选项数组常量：命中", () => {
    expect(findModuleLoadT('const OPTS = [{ label: t("a") }, { label: t("b") }];')).toHaveLength(1);
  });

  // computed(() => t(…)) 遍地都是，放过它是这条判据存在的全部理由：回调是函数类节点。
  it("computed 里的 t()：不命中", () => {
    expect(findModuleLoadT('const X = computed(() => t("a"));')).toEqual([]);
  });

  it("箭头函数常量：不命中", () => {
    expect(findModuleLoadT('const X = () => t("a");')).toEqual([]);
  });

  it("ref 立即求值：命中", () => {
    expect(findModuleLoadT('const X = ref(t("a"));')).toHaveLength(1);
  });

  it("withDefaults 的默认值：命中", () => {
    expect(findModuleLoadT('const props = withDefaults(defineProps<Props>(), { label: t("a") });')).toHaveLength(1);
  });

  // 未赋值的宏调用同样在加载期求值（仓库里真有这种写法），它不是声明，靠判据本身命中。
  it("未赋值的 withDefaults 调用：命中", () => {
    expect(findModuleLoadT('withDefaults(defineProps<Props>(), { label: t("a") });')).toHaveLength(1);
  });

  it("withDefaults 里写成箭头函数的默认值：不命中", () => {
    expect(
      findModuleLoadT('const props = withDefaults(defineProps<Props>(), { label: () => t("a") });')
    ).toEqual([]);
  });

  it("函数体内的 t()：不命中", () => {
    expect(findModuleLoadT('function f() { return t("a"); }')).toEqual([]);
    expect(findModuleLoadT('function f() {\n  const x = t("a");\n}')).toEqual([]);
  });

  // 缺陷①，方向最坏的一类：**在合法代码上报出**。旧实现在 depth-0 换行处收段，而 `=>` 之后
  // 紧跟的换行正是这种情况，切出的后半段段首是 `t`，不在续行守卫的记号表里，于是 `t("…")`
  // 单独成段、被当成调用语句命中——而那一整段是含 `=>` 的，违反旧判据自己写下的「整段不含
  // `=>`」。本仓以 `=>` 结尾的行有 74 行，空体多行箭头是普通写法。
  // AST 判据下箭头函数是函数类节点，回调体天然排除，不需要任何续行守卫。
  it("空体多行箭头不命中（缺陷①：旧实现在合法代码上误报）", () => {
    expect(findModuleLoadT('const formatRowLabel = (row: { name: string }) =>\n  t("a");\n')).toEqual([]);
    expect(findModuleLoadT('const f = (a) =>\n  t("a")')).toEqual([]);
  });

  // 缺陷②：段首不是候选的语句被整段丢弃。旧实现只把「声明」与「调用语句」当候选段首，
  // `arr.push(…)` / `store.label = …` / `obj[k] = …` / `new Foo(…)` / `typeof …` 都不是候选，
  // 整段连同其中的加载期 t() 一起被丢掉。旧实现的检出还依赖分号风格（前一条语句带 `;` 时
  // 它也漏），故这个回归正好落在无分号风格上——即 useStreamingLoad.ts / useStreamingCollection.ts。
  // AST 判据只看「调用在不在函数体里」，与语句长什么样无关。
  it("段首不是候选的语句同样命中（缺陷②：旧实现整段丢弃）", () => {
    expect(findModuleLoadT('const A = 1\narr.push(t("a"))\n')).toHaveLength(1);
    expect(findModuleLoadT('const store = {}\nstore.label = t("a")\n')).toHaveLength(1);
    expect(findModuleLoadT('obj[k] = t("a")\n')).toHaveLength(1);
    expect(findModuleLoadT('const A = 1\nnew Foo(t("a"))\n')).toHaveLength(1);
    expect(findModuleLoadT('const A = 1\nconst x = typeof t("a")\n')).toHaveLength(1);
  });

  // 对象字面量的属性不是函数类节点，只有箭头回调本身是：混合形态只放过箭头那半边。
  // 旧实现按「整段不含 `=>`」一刀切，这里的 t 确实在加载期求值却被整段放过。
  it("混合形态里的 t() 照常命中（旧实现整段放过）", () => {
    expect(findModuleLoadT('const X = { a: () => 1, b: t("c") };')).toHaveLength(1);
    expect(findModuleLoadT('const X = { m() { return t("a") } };')).toEqual([]);
  });

  // 控制流块不是函数体：`if (c) { init(t("a")) }` 里的 t 在模块加载时求值一次。
  // 旧实现按段首关键字（`if` / `for` / `while` …）整段放过，因为它的段首判据分不开控制流块
  // 与嵌在其中的函数/类声明——同样是假阴性。
  it("控制流块里的 t() 命中（旧实现按段首关键字放过）", () => {
    expect(findModuleLoadT('if (flag) {\n  initLabels(t("a"));\n}')).toHaveLength(1);
    expect(findModuleLoadT('for (const x of xs) { initLabels(t("a")); }')).toHaveLength(1);
  });

  // 类成员的求值点分两种，**由运行时定出来的**（不是从 AST 形状推的）：把片段真的跑一遍、
  // 看副作用落在「类定义结束」之前还是之后——
  //   类定义时求值（与模块顶层同属只求值一次，报）：`static` 字段的初始化式、`static {}` 块、
  //   以及字段名里的**计算键**（`class A { [t("a")] = 1 }` 里的 t 在类定义时求值）。
  //   构造时求值（每次 new 都重算，不报）：**实例字段的初始化式**——实测顺序是
  //   「类定义结束 → 实例字段」，故它与函数体同类。报它就是在合法代码上 exit 1。
  // 旧实现整段放过 `class`（段首判据分不开这两类形态），故这里只保留 static 那半边是 RED。
  it("static 字段 / static 块 / 计算键 / 计算方法名命中，实例字段与实例方法不命中", () => {
    expect(findModuleLoadT('class A { static x = t("a") }')).toHaveLength(1);
    expect(findModuleLoadT('class A { static { init(t("a")) } }')).toHaveLength(1);
    expect(findModuleLoadT('class A { [t("a")] = 1 }')).toHaveLength(1);
    expect(findModuleLoadT('class A { static [t("a")] = 1 }')).toHaveLength(1);
    // 实例字段的初始化式在构造时求值，不报；同一个字段的计算键照报。
    expect(findModuleLoadT('class A { x = t("a") }')).toEqual([]);
    expect(findModuleLoadT('class A { x = () => t("a") }')).toEqual([]);
    expect(findModuleLoadT('class A { m() { return t("a") } }')).toEqual([]);
    // 嵌套类同理：外层是实例字段时，内层的 static 字段也要等到外层被构造才求值。
    expect(findModuleLoadT('class A { x = class B { static y = t("a") } }')).toEqual([]);
  });

  // 计算方法名与计算字段键是同一族里求值点相同的两种形态，必须一起处置：字段键报了而方法名
  // 不报是说不通的（同一个 `[t("a")]` 前缀，两种写法）。方法体照旧是懒的。
  it("方法 / 访问器的计算名命中，方法体不命中", () => {
    expect(findModuleLoadT('class A { [t("a")]() {} }')).toHaveLength(1);
    expect(findModuleLoadT('class A { static [t("a")]() {} }')).toHaveLength(1);
    expect(findModuleLoadT('class A { get [t("a")]() { return 1 } }')).toHaveLength(1);
    expect(findModuleLoadT('const X = { [t("a")]() {} };')).toHaveLength(1);
    expect(findModuleLoadT('const X = { [t("a")]: 1 };')).toHaveLength(1);
    expect(findModuleLoadT('class A { m() { return t("a") } }')).toEqual([]);
    // 只断言 toHaveLength(1) 分不开「名字命中」与「体命中」——一个语句只报一条，两种都是 1。
    // 这两条一起才能分开：名字里有 t、体里也有 → 仍只报 1 条（体没有额外贡献）；
    // 名字里没有 t、只有体里有 → 不报（体本身是懒的）。
    expect(findModuleLoadT('class A { [t("a")]() { return t("b") } }')).toHaveLength(1);
    expect(findModuleLoadT('class A { [k]() { return t("a") } }')).toEqual([]);
  });

  // 字段装饰器在类定义时求值，与 static 字段的装饰器同一结论。此前实例字段被 early return
  // 整条跳过，于是 `@dec(t("a")) x = 1` 不报、`@dec(t("a")) static x = 1` 报——同一个装饰器
  // 写法因 static 与否而结论相反。字段的各种形态一起钉住。
  it("字段装饰器命中，字段各形态一致", () => {
    expect(findModuleLoadT('class A { @dec(t("a")) x = 1; }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) static x = 1; }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) accessor x = 1; }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) [k] = 1; }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) #x = 1; }')).toHaveLength(1);
    // 初始化式照旧是懒的：装饰器里有 t、初始化式里也有 → 仍只报 1 条。
    expect(findModuleLoadT('class A { @dec(t("a")) x = t("b"); }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(k) x = t("a"); }')).toEqual([]);
  });

  // 值就是其操作数的几种包装：剥掉之后运行时调用的就是 t。`t!` / `as` / `<T>` / `satisfies`
  // 发射出来就是 `t("a")`（实测 transpileModule 输出逐字相同），赋值表达式的值取右操作数。
  it("类型层包装与赋值包装照常命中", () => {
    expect(findModuleLoadT('const X = t!("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = (t as any)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = (<any>t)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = (t satisfies any)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = (t = t)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = (t!)("a");')).toHaveLength(1);
  });

  // 按真值取值的包装刻意不剥：调的是 t 还是 obj.f 取决于 t 在运行时是不是真值。
  it("按真值取值的包装不剥（已记名的假阴性）", () => {
    expect(findModuleLoadT('const X = (t || obj.f)("a");')).toEqual([]);
    expect(findModuleLoadT('const X = (t ?? obj.f)("a");')).toEqual([]);
  });

  // 方法 / 访问器装饰器与字段装饰器同属一类：求值点在类定义时，判据报出。它们此前挂在函数类
  // 节点下被整条跳过——与字段装饰器是同一个洞的两半，两半现在都补上了。
  it("方法 / 访问器装饰器命中，方法体不命中", () => {
    expect(findModuleLoadT('class A { @dec(t("a")) m() {} }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) static m() {} }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) get x() { return 1 } }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(t("a")) set x(v) {} }')).toHaveLength(1);
    // 方法体照旧是懒的：装饰器里有 t、体里也有 → 仍只报 1 条；只有体里有 → 不报。
    expect(findModuleLoadT('class A { @dec(t("a")) m() { return t("b") } }')).toHaveLength(1);
    expect(findModuleLoadT('class A { @dec(k) m() { return t("a") } }')).toEqual([]);
  });

  // 装饰器里**不会被求值**或分模式的形态，逐一记名并钉住，免得被误当成已覆盖：
  // 构造器装饰器 TS 两种模式都不发射（判据不报，一致）；`static {}` 的装饰器同样不发射，
  // 但判据会报——假阳性；参数装饰器只有 legacy 模式求值，标准模式下不存在。
  it("装饰器的三种记名形态", () => {
    expect(findModuleLoadT('class A { @dec(t("a")) constructor() {} }')).toEqual([]);
    expect(findModuleLoadT('class A { @dec(t("a")) static {} }')).toHaveLength(1);
    expect(findModuleLoadT('class A { constructor(@dec(t("a")) x) {} }')).toEqual([]);
    expect(findModuleLoadT('@dec(t("a"))\nclass A {}')).toHaveLength(1);
  });

  // 调用目标外面包着括号或逗号表达式时，运行时调用的仍是 t，判据先剥掉再认。
  it("(t)(…) / (0, t)(…) / new (t)(…) 照常命中", () => {
    expect(findModuleLoadT('const X = (t)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = (0, t)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = new (t)("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = ((t))("a");')).toHaveLength(1);
    // 剥完之后不是裸标识符 t 的照旧不认。
    expect(findModuleLoadT('const X = (0, obj.t)("a");')).toEqual([]);
  });

  // 标签模板同样调用 t，但不是本仓的 t 调用形态，故不收（见 statementHasEagerTranslationCall）。
  it("标签模板不报（已记名的假阴性）", () => {
    expect(findModuleLoadT("const X = t`a`;")).toEqual([]);
  });

  // 假阴性列表点名的是**一整类**：函数类节点在模块求值期间被调用。IIFE 只是最显眼的一种，
  // 下面两种漏报机理相同，故一并钉住——免得下次有人以为「修了 IIFE 就收敛了」。
  it("函数体被调用时漏报：IIFE 与先声明后调用同属一类（已记名的假阴性）", () => {
    expect(findModuleLoadT('(() => t("a"))();')).toEqual([]);
    expect(findModuleLoadT('(function () { return t("a") })();')).toEqual([]);
    expect(findModuleLoadT('function f() { return t("a"); }\nf();')).toEqual([]);
    expect(findModuleLoadT('const f = () => t("a");\nf();')).toEqual([]);
  });

  // 模板字面量插值里的是代码：旧实现把字面量整体置空，插值里的调用看不见。
  it("模板字面量插值里的 t() 命中（旧实现置空整段字面量）", () => {
    expect(findModuleLoadT('const X = `${t("a")}`;')).toHaveLength(1);
  });

  // 一条语句只报一次，行号是语句起点、snippet 是那条语句的原文（含末尾分号）：读者要改的是
  // 整条语句，逐处列举会把同一处缺陷刷成好几条，也会把 snippet 切碎到看不出上下文。
  it("报出行号与语句原文，一条语句只报一次", () => {
    expect(findModuleLoadT('const A = 1;\nconst OPTS = [\n  { label: t("a") },\n];')).toEqual([
      { line: 2, snippet: `const OPTS = [ { label: t("a") }, ];` },
    ]);
    expect(findModuleLoadT('const OPTS = [{ label: t("a") }, { label: t("b") }];')).toHaveLength(1);
  });

  it("注释里的 t() 不算调用", () => {
    expect(findModuleLoadT('// const X = t("a");\n/* const Y = t("b"); */')).toEqual([]);
  });

  // 字符串与正则里的 t( 是数据不是调用。规则 5 的扫描器不认字符串边界，故把调用示例当测试数据
  // 会被它当成引用；本规则走 AST，没有这条已知误报面。
  it("字符串与正则里的 t( 不算调用", () => {
    expect(findModuleLoadT('const S = "t(";')).toEqual([]);
    expect(findModuleLoadT('const S = \'const X = t("a")\';')).toEqual([]);
    expect(findModuleLoadT('const re = /t\\(/;\nconst X = [{ l: t("a") }];')).toHaveLength(1);
    expect(findModuleLoadT('const re = /{/;\nfunction f() { const x = t("a"); }')).toEqual([]);
  });

  // 只认裸标识符 t，与规则 5 同一处收窄：属性访问与 `$t(` 不在范围内（本仓的 t 一律从 src/i18n
  // 具名导入）。
  it("属性访问与 $t( 不算", () => {
    expect(findModuleLoadT('const X = obj.t("a");')).toEqual([]);
    expect(findModuleLoadT('const X = $t("a");')).toEqual([]);
  });

  // 类型字面量与泛型实参不影响判定：AST 不靠深度记账，它们天然是数据。
  it("类型字面量与泛型实参不影响判定", () => {
    expect(findModuleLoadT('const X: { a: string } = t("b");')).toHaveLength(1);
    expect(findModuleLoadT('const X = ref<{ a: string }>(t("a"));')).toHaveLength(1);
    expect(findModuleLoadT('const X: {\n  a: string;\n} = t("b");')).toHaveLength(1);
  });

  // 语句排布与判据无关：`}` 结尾的语句、`export interface`、无分号风格都不影响。
  it("语句排布不影响判定", () => {
    expect(findModuleLoadT('interface I { a: string }\nconst X = [{ l: t("a") }];')).toHaveLength(1);
    expect(findModuleLoadT('function f() {}\nconst X = [{ l: t("a") }];')).toHaveLength(1);
    expect(findModuleLoadT('if (x) { y(); }\nconst X = [{ l: t("a") }];')).toHaveLength(1);
    expect(findModuleLoadT('const a = t("x.y")\nconst b = computed(() => 1)\n')).toHaveLength(1);
    expect(findModuleLoadT('const b = computed(() => 1)\nconst a = t("x.y")\n')).toHaveLength(1);
    expect(
      findModuleLoadT('const OPTS = [{ label: t("common.a") }]\nconst PICKER = computed(() => 1)\n')
    ).toHaveLength(1);
  });

  // 跨行语句不再靠「续行首字符」猜边界：换行处的记号是 `.` / `?` / `&&` 还是 `(` / `[` / `+` /
  // `-` / `~` / `/` / `,`，判据都只看这条语句里有没有加载期调用。旧实现为此维护了一张记号表，
  // 表里刻意不收的那几个记号各留下一类假阴性。
  it("跨行语句照常命中，与续行记号无关", () => {
    for (const cont of [".bar", "?.bar", "&&", "||", "??"]) {
      expect(findModuleLoadT(`const X = foo\n  ${cont}(t("a"));`), cont).toHaveLength(1);
    }
    expect(findModuleLoadT('const X = c\n  ? t("a")\n  : t("b");')).toHaveLength(1);
    expect(findModuleLoadT('const X = foo\n\n  .bar(t("a"));')).toHaveLength(1);
    expect(findModuleLoadT('const X = foo\n  (t("a"));')).toHaveLength(1);
    expect(findModuleLoadT('const X = foo\n  [t("a")];')).toHaveLength(1);
    expect(findModuleLoadT('const X = foo\n  + t("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = foo\n  - t("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = foo\n  / t("a");')).toHaveLength(1);
    expect(findModuleLoadT('foo(),\n  t("a");')).toHaveLength(1);
  });

  // 多行调用的实参里那个 t() 同样是加载期求值：旧实现按「整段含 `=>`」把它放过。
  // 判据只看这个调用在不在函数体里——同一段里别的实参是箭头函数，与它无关。
  it("多行调用的实参命中，同段的箭头回调不影响它", () => {
    expect(findModuleLoadT('const opts = build(\n  t("a"),\n  () => 1\n);')).toHaveLength(1);
    expect(findModuleLoadT('const opts = build(\n  t("a"),\n  () => 1\n)')).toHaveLength(1);
    expect(findModuleLoadT('const OPTS = [\n  { label: t("a") },\n  { label: t("b") },\n]')).toHaveLength(1);
  });

  // 行首的 `.5` 是数值字面量（ASI 在此处成立），能独立成句，不会被并进上一条语句。
  it("行首的数值字面量不被并进上一条语句", () => {
    expect(findModuleLoadT('const X = t("a")\n.5')).toEqual([{ line: 1, snippet: 'const X = t("a")' }]);
  });

  // 解析失败必须响亮：静默跳过是本文件全部六次缺陷的共同形状。判据是「解析不出来就报出来」，
  // 而不是「尽量猜一个结果」——在猜错的语法树上跑规则，报出的行号与命中都不可信。
  // 只有语法错误会落进 parseDiagnostics，类型错误（`const X: number = "s"`）不是本规则的管辖。
  it("语法错误时抛出，不静默返回空", () => {
    expect(() => findModuleLoadT('const X = t("a");\n}\n')).toThrow(/^第 2 行第 1 列：/);
    expect(findModuleLoadT('const X: number = "s";\nconst Y = t("a");')).toHaveLength(1);
  });

  // 廉价预过滤：正文里没有独立的 `t` 标识符就不进解析器。调用点必然含它，故不会漏掉命中——
  // 代价只是这类文件里的语法错误也不会由本函数报出来（本规则只对 t() 负责）。
  it("没有独立 t 标识符的正文不进解析器", () => {
    expect(findModuleLoadT("const a = }\n")).toEqual([]);
    expect(findModuleLoadT("")).toEqual([]);
    // `$t(` 不是裸标识符 t，预过滤把它挡在解析器外——判据本身也不认它，结论一致。
    expect(findModuleLoadT('const X = $t("a");')).toEqual([]);
  });

  // 预过滤**不能**只看 `t(` 这三个连续字符：下面四种写法都调用 t、都解析成同一个调用表达式，
  // 却都不含 `t(` 这个连续子串。只看 `t(` 时它们整份文件不进解析器，闸门一声不响。
  it("t 与 ( 之间隔着空白 / 换行 / 块注释，或写成 t?.() 时照常命中", () => {
    expect(findModuleLoadT('const X = t ("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = t\n("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = t /* 说明 */ ("a");')).toHaveLength(1);
    expect(findModuleLoadT('const X = t?.("a");')).toHaveLength(1);
  });

  // `new t(…)` 同样会执行到 t：写在模块顶层就是加载期求值一次，与 `t(…)` 症状一致。
  // 旧实现靠正则 `t\(` 命中它，改用 AST 后若只认 CallExpression 就会把它漏掉。
  it("new t(…) 照常命中", () => {
    expect(findModuleLoadT('const X = new t("a");')).toHaveLength(1);
    expect(findModuleLoadT('const A = 1\nnew Foo(t("a"))\n')).toHaveLength(1);
    // 属性访问不是裸标识符 t，与规则 5 同一处收窄。
    expect(findModuleLoadT('const X = new obj.t("a");')).toEqual([]);
  });

  // 无尾分号的 import 块之后紧跟的模块级声明（两个真实文件整份不写分号）。夹具直接用那两个文件：
  // 合成例子钉不住真实排布（单引号、类型导入混排）。两个文件都是批次 5 的目标，
  // 白名单一撤就该被这条规则保护。
  for (const rel of [
    "src/composables/useStreamingLoad.ts",
    "src/composables/useStreamingCollection.ts",
  ]) {
    it(`${rel} 的 import 块之后的模块级声明命中`, () => {
      const src = readFileSync(join(process.cwd(), rel), "utf8");
      // 夹具前提：文件以不带分号的 import 块开头。前提不成立时下面的断言会直接失败，
      // 而不是悄悄退化成一条测不到东西的用例。
      expect(src).toMatch(/^import[^\n]*[^;\n]\n/);
      const injected = src.replace(/^(?:import[^\n]*\n)+/, '$&const OPTS = [{ label: t("common.a") }];\n');
      expect(injected).not.toBe(src);
      const importLines = src.match(/^(?:import[^\n]*\n)+/)[0].trimEnd().split("\n").length;
      expect(findModuleLoadT(injected)).toEqual([
        { line: importLines + 1, snippet: 'const OPTS = [{ label: t("common.a") }];' },
      ]);
    });
  }
});

describe("findHtmlCjkText", () => {
  it("命中 title 里的中文", () => {
    expect(findHtmlCjkText("<title>SeshBuddy 助手</title>")).toEqual(["助手"]);
  });

  it("命中正文里的中文", () => {
    expect(findHtmlCjkText("<h1>你好</h1>")).toEqual(["你好"]);
  });

  it("忽略 HTML 注释与标签属性", () => {
    expect(findHtmlCjkText('<!-- 中文注释 --><div id="app"></div>')).toEqual([]);
  });

  it("忽略纯英文入口", () => {
    expect(findHtmlCjkText('<html lang="en"><title>SeshBuddy</title></html>')).toEqual([]);
  });
});

describe("findUnsupportedHtmlLang", () => {
  const LANGS = ["en", "zh", "ja", "de"];

  it("受支持的语言标记放行", () => {
    expect(findUnsupportedHtmlLang('<html lang="en">', LANGS)).toBeNull();
  });

  it("区域变体不在受支持列表内", () => {
    expect(findUnsupportedHtmlLang('<html lang="zh-CN">', LANGS)).toBe("zh-CN");
  });

  it("缺少 lang 属性同样报出", () => {
    expect(findUnsupportedHtmlLang("<html>", LANGS)).toBe("缺少 lang 属性");
  });
});

// 闸门是 CI 闸门，通过路径绿了不算数，必须在真实违规上退出非 0。
// 闸门按脚本自身位置反推仓库根，所以把脚本复制进临时目录，整套跑在夹具上。
// 测试文件里的 import.meta.url 被 vitest 改写成了非 file: URL，只能按 cwd 定位
// （npm test 与 CI 都从仓库根启动）。
const SCRIPT = join(process.cwd(), "scripts", "check-i18n.mjs");

/**
 * 夹具自带的英文允许清单（规则 8）。
 *
 * 与 injectFixturePending 同一个理由：**不复制仓库的真实清单**。真实清单是从裁决集派生的
 * 快照，会随每次裁定增减；夹具若读它，用例的前提就挂在仓库数据上——仓库里新裁一条，
 * 就可能让某个用例因与夹具无关的原因通过或失败。形态与 FRONTEND_PENDING 不同：那份是闸门
 * 源码里的常量、只能改写源码；允许清单本就是独立的数据文件，夹具写自己的一份即可。
 *
 * 这些条目是**夹具声明的「已裁定」**，不是随手放行——它们全是本文件其它用例的装置：
 * `save` 是模板正文的占位文案，`{ use` / `50} off` 要一个落单花括号（净 +1 / 净 -1），
 * `Don't save` 要一个撇号，`a \`b` / `a \`b\` c` 要反引号，`C:\Users\x` 要一个反斜杠。
 * 把它们换成中文或删掉，那些用例就失去了装置本身，故只能由夹具声明为「已裁定」。
 */
const FIXTURE_ENGLISH_ALLOWLIST = {
  save: "夹具 .vue 用例的模板占位文案",
  "Don't save": "剥注释自检用例的撇号装置",
  "{ use": "剥注释自检用例的落单花括号装置（净 +1）",
  "50} off": "剥注释自检用例的落单花括号装置（净 -1）",
  "a `b": "跨区反引号用例的落单反引号装置",
  "a `b` c": "模板区自洽反引号用例的装置",
  "C:\\Users\\x": "剥注释扫描器的 `\\` 分支用例的 Windows 路径装置",
};

/** 夹具允许清单的 JSON 文本；`extra` 在夹具基线之上追加条目。 */
function fixtureAllowlist(extra = {}) {
  return `${JSON.stringify({ ...FIXTURE_ENGLISH_ALLOWLIST, ...extra }, null, 2)}\n`;
}

const BASE_FIXTURE = {
  "src/app.ts": 'const a = "save";\n',
  // 白名单里的文件必须**仍有生产 CJK**（棘轮，见 RUST_PENDING_EXCEPTIONS 与 main()）：
  // 夹具的 `src-tauri/src/lib.rs` 在 FIXTURE_RUST_PENDING 里，故基线就带上待迁移的中文。
  // 写成零 CJK 会让基线夹具自己踩上棘轮，每条用例都因一个与它无关的原因变红。
  "src-tauri/src/lib.rs": 'const A: &str = "保存";\n',
  "src-tauri-proxy/src/lib.rs": 'const A: &str = "save";\n',
  "transcript-store/src/lib.rs": 'const A: &str = "save";\n',
  "src/locales/en/common.json": '{ "a": "1" }\n',
  "src/locales/zh/common.json": '{ "a": "x" }\n',
  "src/locales/ja/common.json": '{ "a": "x" }\n',
  "src/locales/de/common.json": '{ "a": "x" }\n',
  "index.html": '<!doctype html>\n<html lang="en">\n<title>SeshBuddy</title>\n',
  "scripts/i18n-english-allowlist.json": fixtureAllowlist(),
  // 规则 9 的允许清单与规则 8 的同一形态：夹具自带一份，闸门读不到就硬失败。
  // 基线夹具的四份语言包只有一个键，构不成任何漂移组，故空表即可。
  "scripts/i18n-drift-allowlist.json": "{}\n",
};

// 需要「路径在白名单里」这一前提的用例共用它。它是**夹具自己的常量**，不是「某个真实文件
// 碰巧还在白名单里」——夹具注入白名单时就把这条路径放进去（见 injectFixturePending）。
// 计划 5 的终态是白名单清空（至多只剩 usePricingCatalog.ts 一条），依赖真实白名单的用例
// 在那个终态下前提根本无法成立。
const PENDING_FILE = "src/composables/useSessions.ts";

const tempRoots = [];

/**
 * 把闸门脚本里的 FRONTEND_PENDING 整段换成夹具自己的列表。
 *
 * 不复制仓库的真实白名单：它只减不增，批次 5 的终态是清空（至多只剩 usePricingCatalog.ts 一条），
 * 那时夹具里就没有任何「仍在白名单里的路径」，依赖它的用例会集体失去前提。由夹具自带列表，
 * 前提就与仓库白名单解耦，清空之后这些用例照样成立。
 *
 * 锚点取 `const FRONTEND_PENDING = [` 全名，收尾取其后的第一个 `];`：紧随其后的是另一个数组
 * RUST_PENDING，只认 `];` 会把它一并吞掉。
 *
 * 锚点没匹配上时**抛出**而不是原样返回：原样写过去，用例会在仓库的真实白名单上跑，
 * 于是可能因一个与夹具无关的原因通过或失败——那种静默失效比这里的响亮失败难查得多。
 */
function injectFixturePending(source) {
  const anchor = "const FRONTEND_PENDING = [";
  const start = source.indexOf(anchor);
  if (start === -1) throw new Error("夹具注入失败：脚本里找不到 FRONTEND_PENDING 的锚点");
  const end = source.indexOf("];", start);
  if (end === -1) throw new Error("夹具注入失败：FRONTEND_PENDING 的数组没有收尾");
  const items = [PENDING_FILE].map((p) => `\n  ${JSON.stringify(p)},`).join("");
  return source.slice(0, start) + `${anchor}${items}\n];` + source.slice(end + 2);
}

/**
 * 需要「路径在 RUST_PENDING 里」这一前提的用例共用它。与 PENDING_FILE 同理，是**夹具自己的
 * 常量**：真实清单是仓库的待迁移欠债，计划 8 的 T1 把它从目录条目改成了逐文件条目，
 * 目录条目原本碰巧能匹配夹具里的 `src-tauri/src/lib.rs`，逐文件条目一个都匹配不上——
 * 夹具若继续读真实清单，「跳过的文件数要出现在 CI 日志里」那条用例会因一个与夹具无关的
 * 原因失败。自带一份，前提就与仓库清单解耦。
 *
 * 只列一个文件：那条用例断言的是 `1/3`（三个扫描根各一个夹具文件）。
 */
const FIXTURE_RUST_PENDING = ["src-tauri/src/lib.rs"];

/**
 * 把闸门脚本里的 RUST_PENDING 整段换成夹具自己的列表。锚点取全名 `const RUST_PENDING = [`，
 * 收尾取其后的第一个 `];`——紧随其后的是 RUST_UNTRANSLATED，只认 `];` 会把它一并吞掉。
 * 锚点没匹配上时抛出而不是原样返回，理由同 injectFixturePending。
 *
 * `entries` 默认取 FIXTURE_RUST_PENDING；棘轮里「条目指向的文件不存在」那条用例传自己的列表
 * （它要的是一条**失效**条目，与默认列表的前提相反）。
 */
function injectFixtureRustPending(source, entries = FIXTURE_RUST_PENDING) {
  const anchor = "const RUST_PENDING = [";
  const start = source.indexOf(anchor);
  if (start === -1) throw new Error("夹具注入失败：脚本里找不到 RUST_PENDING 的锚点");
  const end = source.indexOf("];", start);
  if (end === -1) throw new Error("夹具注入失败：RUST_PENDING 的数组没有收尾");
  const items = entries.map((p) => `\n  ${JSON.stringify(p)},`).join("");
  return source.slice(0, start) + `${anchor}${items}\n];` + source.slice(end + 2);
}

/**
 * 把闸门脚本里的 `RUST_PENDING_EXCEPTIONS`（具名例外表）整段换成夹具给的条目。
 *
 * 与 injectFixtureRustPending 同理，是**夹具自己的**条目而不是仓库的真实例外表：真实表是
 * 各批迁移时逐条写进去的裁定，会随批次增减，用例的前提不能挂在它上面。
 *
 * 默认空表：基线夹具的 `src-tauri/src/lib.rs` 有生产 CJK，棘轮不需要例外。
 * 只有棘轮的用例需要它（②把文件写进例外表 → 放行；④例外表本身让规则 2 放行）。
 * 锚点取全名，收尾取其后的第一个 `];`；锚点没匹配上时抛出而不是原样返回，理由同 injectFixturePending。
 */
function injectFixtureRustPendingExceptions(source, entries) {
  const anchor = "const RUST_PENDING_EXCEPTIONS = [";
  const start = source.indexOf(anchor);
  if (start === -1) throw new Error("夹具注入失败：脚本里找不到 RUST_PENDING_EXCEPTIONS 的锚点");
  const end = source.indexOf("];", start);
  if (end === -1) throw new Error("夹具注入失败：RUST_PENDING_EXCEPTIONS 的数组没有收尾");
  const items = entries.map((p) => `\n  ${JSON.stringify(p)},`).join("");
  return source.slice(0, start) + `${anchor}${items}\n];` + source.slice(end + 2);
}

function buildFixture(overrides = {}, { rustPending = FIXTURE_RUST_PENDING, rustPendingExceptions = [] } = {}) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "i18n-gate-")));
  tempRoots.push(root);
  for (const [rel, content] of Object.entries({ ...BASE_FIXTURE, ...overrides })) {
    const full = join(root, rel);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, content);
  }
  mkdirSync(join(root, "scripts"), { recursive: true });
  writeFileSync(
    join(root, "scripts/check-i18n.mjs"),
    injectFixtureRustPendingExceptions(
      injectFixtureRustPending(injectFixturePending(readFileSync(SCRIPT, "utf8")), rustPending),
      rustPendingExceptions
    )
  );
  // 闸门现在依赖 typescript（规则 7 的 AST 遍历）。裸标识符按**导入方所在位置**解析，脚本被
  // 复制到临时目录后 Node 会从那里向上找 node_modules——找不到，闸门会以模块解析失败退出非 0，
  // 而那不是被测对象。夹具因此把依赖一并供给，让闸门在夹具里与在仓库里跑的是同一条路径。
  symlinkSync(join(process.cwd(), "node_modules"), join(root, "node_modules"), "dir");
  return root;
}

function runScript(scriptPath) {
  const r = spawnSync(process.execPath, [scriptPath], { encoding: "utf8" });
  return { code: r.status, stdout: r.stdout, stderr: r.stderr };
}

function runGate(overrides = {}, fixtureOptions = {}) {
  return runScript(join(buildFixture(overrides, fixtureOptions), "scripts", "check-i18n.mjs"));
}

afterAll(() => {
  for (const root of tempRoots) rmSync(root, { recursive: true, force: true });
});

describe("check-i18n.mjs 退出码", () => {
  it("干净夹具退出 0", () => {
    const { code, stdout } = runGate();
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  it("前端新增硬编码中文时退出非 0 并指名文件", () => {
    const { code, stderr } = runGate({ "src/app.ts": 'const a = "保存";\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src/app.ts: 保存");
  });

  it("Vue 模板文本节点里的中文退出非 0", () => {
    const { code, stderr } = runGate({ "src/Comp.vue": "<template><div>保存</div></template>\n" });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文（模板文本）src/Comp.vue: 保存");
  });

  it("console.* 的日志文案不报（规则 1 的日志排除）", () => {
    const { code } = runGate({ "src/app.ts": 'console.warn("读取界面语言失败", err);\n' });
    expect(code).toBe(0);
  });

  // 中文标点单独出现时同样是硬编码文案：`join("、")` 在英文界面上渲染成 `Claude、Gemini`，
  // 而它一个汉字都不含，只测汉字的实现会放它过去。
  it("只有中文标点的硬编码同样退出非 0", () => {
    const { code, stderr } = runGate({ "src/app.ts": 'const a = "、";\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src/app.ts: 、");
  });

  // 字符串里的 `/*` 曾被当成块注释起始，把其后真实代码一路抹到下一个 `*/`：
  // 落在里面的硬编码中文对规则 1 完全不可见，闸门照常报「通过」。
  it("字符串里的 /* 之后的硬编码中文不再被静默跳过", () => {
    const { code, stderr } = runGate({
      "src/app.ts": 'const a = "/* not a comment";\nconst b = "保存";\n/* c */\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src/app.ts: 保存");
  });

  // `++` / `--` 之后的 `/` 曾被当成正则起始：`i++ / 2; // 显示 "保存" 按钮` 里的注释起始
  // 被吞掉，注释正文变成代码，规则 1 在一行正确代码上误报——闸门一旦乱叫就会被调松。
  it("自增自减之后的注释不被误报为硬编码", () => {
    const { code } = runGate({ "src/app.ts": 'let i = 0;\ni++ / 2; // 显示 "保存" 按钮\n' });
    expect(code).toBe(0);
  });

  // 正则字面量里的引号曾让引号配对错位，**该文件其余部分再也不被扫描**：src/utils/json.ts
  // 第 9 行的 `.replace(/"/g, "&quot;")`（行号在 `987b91a0` 上量得）就是真实写法，末尾新写的硬编码中文一条都不报。
  // 这不是「可能误报」，是静默失明——比误报严重，因为它让闸门看起来守住了。
  it("正则字面量里的引号不再让文件其余部分失明", () => {
    const { code, stderr } = runGate({
      "src/app.ts": 'const re = /"/g;\nconst a = s.replace(/"/g, "&quot;");\nconst b = "保存";\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src/app.ts: 保存");
  });

  // 后端日志宏的排除必须是承重的：src-tauri-proxy/src/lib.rs 不在白名单也不在刻意排除里，
  // 闸门真的会读它（下面那条正面用例钉住这点），所以这里的 exit 0 只能由日志排除解释。
  it("后端日志宏的文案不报（规则 2 的日志排除）", () => {
    const { code } = runGate({ "src-tauri-proxy/src/lib.rs": 'tracing::warn!("读取语言失败: {}", e);\n' });
    expect(code).toBe(0);
  });

  it("后端非白名单文件里的普通字面量照旧报出", () => {
    const { code, stderr } = runGate({ "src-tauri-proxy/src/lib.rs": 'const A: &str = "保存";\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src-tauri-proxy/src/lib.rs: 保存");
  });

  // workspace 有三个 crate，扫描根漏掉一个就等于给那个 crate 开了永久后门。
  // 它不在任何白名单前缀下，闸门真的会读它，故这里的 exit 1 只能由扫描根覆盖解释。
  it("transcript-store 里的普通字面量同样报出", () => {
    const { code, stderr } = runGate({ "transcript-store/src/store.rs": 'const A: &str = "保存";\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 transcript-store/src/store.rs: 保存");
  });

  // assistant.rs 是刻意不翻译（助手工具接口，模型读、用户不读），不是待迁移欠债；
  // 它含 12 条非测试中文，若排除失效这条用例会立刻变红。
  it("刻意不翻译的文件放行", () => {
    const { code } = runGate({ "src-tauri-proxy/src/assistant.rs": 'const A: &str = "成功";\n' });
    expect(code).toBe(0);
  });

  // 跳过的文件数必须出现在 CI 日志里，否则闸门看起来守住了后端、实际只查了一小部分。
  // 提示点名扫描根：比例的分母是扫描根之和，说「后端」会让 76 与真实的 79 个文件对不上。
  it("后端白名单跳过时打印扫描根与覆盖比例", () => {
    const { code, stdout } = runGate();
    expect(code).toBe(0);
    expect(stdout).toContain("rule 2：扫描根 src-tauri/src、src-tauri-proxy/src、transcript-store/src 下 1/3 个文件待迁移（白名单跳过）");
  });

  // 与规则 2 同理：没有任何 .vue 被检查过时模板文本分支一个文件都没查过。
  // 批次 5 起白名单逐文件列举，夹具里放不下「被跳过的 .vue」——本仓白名单已只剩 .ts，
  // 树里一个 .vue 都没有时 vueChecked 同样为 0，走的是同一个分支。
  it("模板文本分支没查过任何文件时打印提示", () => {
    const { code, stdout } = runGate({ [PENDING_FILE]: "<template>save</template>\n" });
    expect(code).toBe(0);
    expect(stdout).toContain("rule 1：前端 1/2 个文件待迁移（白名单跳过）");
    expect(stdout).toContain("Vue 模板文本分支未检查任何文件");
  });

  // 提示的触发条件取「有文件被跳过」而非「一个都没查」：批次 2 起每批都会移出若干 .vue，
  // 用后者会让提示在仍有大批文件被跳过时提前静默——这正是它要防的那种假绿灯。
  it("Vue 已被检查但仍有文件待迁移时照样打印覆盖比例", () => {
    const { code, stdout } = runGate({
      [PENDING_FILE]: "<template>save</template>\n",
      "src/components/Clean.vue": "<template>save</template>\n",
    });
    expect(code).toBe(0);
    expect(stdout).toContain("rule 1：前端 1/3 个文件待迁移（白名单跳过）");
    expect(stdout).not.toContain("Vue 模板文本分支未检查任何文件");
  });

  // 入口守卫用 realpath 比较 argv[1] 与 import.meta.url：经由软链接路径调用时二者写法不同，
  // 曾经导致 main() 不执行、闸门无输出地退出 0。这是本任务唯一的静默通过路径，钉住它。
  it("经由软链接路径调用时仍然生效", () => {
    const root = buildFixture({ "src/app.ts": 'const a = "保存";\n' });
    const linkParent = realpathSync(mkdtempSync(join(tmpdir(), "i18n-link-")));
    tempRoots.push(linkParent);
    const link = join(linkParent, "linked");
    symlinkSync(root, link);

    const { code, stderr } = runScript(join(link, "scripts", "check-i18n.mjs"));
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src/app.ts: 保存");
  });

  it("语言包缺口时退出非 0 并列出缺的 key", () => {
    const { code, stderr } = runGate({
      "src/locales/en/common.json": '{ "a": "1", "b": "2" }\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包缺口 ja/common.json: b");
  });

  // 只走 en 的键是半个检查：目标语言里多出来的键静默通过，等于允许语言包单方面长胖。
  it("目标语言多出来的 key 同样退出非 0", () => {
    const { code, stderr } = runGate({
      "src/locales/ja/common.json": '{ "a": "x", "z": "y" }\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包多余键 ja/common.json: z");
  });

  // 命名空间只从 en 目录枚举时，只存在于非 en 包里的命名空间文件根本不会被打开——
  // 而这是语言包形状的唯一守卫。
  it("只存在于非 en 包里的命名空间被打开并报出", () => {
    const { code, stderr } = runGate({ "src/locales/zh/extra.json": '{ "a": "x" }\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包缺口 en/extra.json: 命名空间文件缺失（zh 中存在）");
  });

  it("只存在于 en 包里的命名空间报出目标语言缺文件", () => {
    const { code, stderr } = runGate({ "src/locales/en/only.json": '{ "a": "1" }\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包缺口 zh/only.json: 命名空间文件缺失");
  });

  // 形状不一致（一侧是文本、另一侧是子键集合）曾让反向比对抛 TypeError。
  // 抛出会中断 main()，已积累的 failures 一条都打不出来——闸门静默丢弃整轮诊断。
  // 两个方向都要报，且必须报成「形状不一致」而不是「缺口 / 多余键」：
  // 键两边都在，只是类型不同，报成缺口会把读者引向错误的方向。
  it("en 是文本、目标语言是子键集合时报形状不一致", () => {
    const { code, stderr } = runGate({
      "src/locales/ja/common.json": '{ "a": { "b": "2" } }\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包形状不一致 ja/common.json 与 en/common.json: a");
  });

  it("en 是子键集合、目标语言是文本时报形状不一致", () => {
    const { code, stderr } = runGate({
      "src/locales/en/common.json": '{ "a": { "b": "1" } }\n',
      "src/locales/zh/common.json": '{ "a": "x" }\n',
      "src/locales/ja/common.json": '{ "a": { "b": "y" } }\n',
      "src/locales/de/common.json": '{ "a": { "b": "z" } }\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包形状不一致 zh/common.json 与 en/common.json: a");
  });

  // 崩溃最坏的后果不是「少报一条」，是「同一轮里其它违规全部消失」。
  // 这条用例把形状不一致与另一处无关违规放进同一次运行，断言两条都在。
  it("形状不一致不吞掉同一轮里的其它违规", () => {
    const { code, stderr } = runGate({
      "src/locales/ja/common.json": '{ "a": { "b": "2" } }\n',
      "src/app.ts": 'const a = "保存";\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包形状不一致 ja/common.json 与 en/common.json: a");
    expect(stderr).toContain("硬编码中文 src/app.ts: 保存");
    expect(stderr).not.toContain("TypeError");
  });

  // 形状对不上的只有那一个路径，同一命名空间里其它键的缺口是各自独立的发现。
  // 按命名空间整份跳过就会把它静默丢弃——正是本文件已经修过两次的那类缺陷。
  it("形状不一致不吞掉同一命名空间里其它键的缺口", () => {
    const { code, stderr } = runGate({
      "src/locales/en/common.json": '{ "a": "1", "c": "3" }\n',
      "src/locales/ja/common.json": '{ "a": { "b": "2" } }\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包形状不一致 ja/common.json 与 en/common.json: a");
    expect(stderr).toContain("语言包缺口 ja/common.json: c");
    // 形状那一个路径改由「形状不一致」报出，不再重复出现在缺口/多余键里。
    expect(stderr).not.toContain("语言包多余键 ja/common.json: a");
    expect(stderr).not.toContain("语言包缺口 ja/common.json: a");
  });

  // 镜像方向同理：en 是子键集合、目标语言是文本，且目标语言另缺一个键。
  it("形状不一致的镜像方向同样不吞掉其它键的缺口", () => {
    const { code, stderr } = runGate({
      "src/locales/en/common.json": '{ "a": { "b": "1" }, "c": "3" }\n',
      "src/locales/ja/common.json": '{ "a": "x" }\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包形状不一致 ja/common.json 与 en/common.json: a");
    expect(stderr).toContain("语言包缺口 ja/common.json: c");
  });

  // 形状一致时不该被这条新检查打扰：值不同是翻译，不是形状问题。
  it("形状一致而取值不同时照旧放行", () => {
    const { code } = runGate({
      "src/locales/ja/common.json": '{ "a": { "b": "y" } }\n',
      "src/locales/zh/common.json": '{ "a": { "b": "z" } }\n',
      "src/locales/de/common.json": '{ "a": { "b": "w" } }\n',
      "src/locales/en/common.json": '{ "a": { "b": "1" } }\n',
    });
    expect(code).toBe(0);
  });

  // changelog.ts 里的 key 是数据，规则 5 的 t("字面量") 扫描看不见它们；
  // 一个错别字会把点分路径原样渲染到 AboutView 与 WhatsNewDialog 上，无人拦。
  it("changelog.ts 里指向不存在 key 的条目退出非 0", () => {
    const { code, stderr } = runGate({
      "src/changelog.ts": 'export const changelog = [{ changeKeys: ["common.b"] }];\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("数据里的翻译 key 不存在 src/changelog.ts:1: common.b");
  });

  it("changelog.ts 里指向子键集合的 key 退出非 0", () => {
    const { code, stderr } = runGate({
      "src/changelog.ts": 'export const changelog = [{ changeKeys: ["common"] }];\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("数据里的翻译 key 指向子键集合 src/changelog.ts:1: common");
  });

  it("changelog.ts 里的 key 都能解析到叶子时放行", () => {
    const { code } = runGate({
      "src/changelog.ts": 'export const changelog = [{ version: "0.1.0", changeKeys: ["common.a"] }];\n',
    });
    expect(code).toBe(0);
  });

  // 字符串里的 `/*` 曾让 Rust 侧的剥注释把其后真实代码一路抹到下一个结束符：
  // 落在里面的硬编码中文对规则 2 完全不可见，闸门照常报「通过」。
  it("Rust 字符串里的 /* 之后的硬编码中文退出非 0", () => {
    const { code, stderr } = runGate({
      "src-tauri-proxy/src/lib.rs":
        'fn a() {\n    let re = "a /* b";\n    let c = "加载中";\n    /* real comment */\n    let d = 2;\n}\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src-tauri-proxy/src/lib.rs: 加载中");
  });

  // 夹具内容必须是规则 1 真的会报出的形状（字符串字面量里的中文）：`<template>保存</template>`
  // 这类裸文本只对 .vue 的模板文本分支有效，放进 .ts 夹具里闸门一声不响，用例就退化成
  // 「无论是否在白名单里都通过」——那样它守不住任何东西。
  it("白名单路径的存量中文放行", () => {
    const { code } = runGate({ [PENDING_FILE]: 'const a = "保存";\n' });
    expect(code).toBe(0);
  });

  it("HTML 标题里的中文退出非 0", () => {
    const { code, stderr } = runGate({
      "index.html": '<!doctype html>\n<html lang="en">\n<title>SeshBuddy 助手</title>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 index.html: 助手");
  });

  it("HTML 语言标记不受支持时退出非 0", () => {
    const { code, stderr } = runGate({
      "index.html": '<!doctype html>\n<html lang="zh-CN">\n<title>SeshBuddy</title>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言标记不受支持 index.html: zh-CN");
  });
});

// 棘轮：`RUST_PENDING` 的条目没有任何东西保证它仍然必要。计划 8 结束时就有两条已零 CJK 的条目
// 留在表里（最终评审发现）——那是 T1 修掉的那类缺陷的**镜像**：文件已经迁完，条目却还留着，
// 于是规则 2 从此看不见它，之后写进去的中文无人过问。判据因此是「每个条目的文件必须仍有生产 CJK」，
// 零 CJK 就报出来；例外只给**有具名非 CJK 义务**的文件（`RUST_PENDING_EXCEPTIONS`）。
describe("check-i18n.mjs 棘轮：白名单条目必须仍然必要", () => {
  // ① 零 CJK 且无义务的条目：夹具的 lib.rs 去掉中文，白名单里还留着它 → 报出并点名。
  it("① 白名单条目的文件已无生产 CJK 且无具名义务 → 退出非 0 并点名该文件", () => {
    const { code, stderr } = runGate({ "src-tauri/src/lib.rs": 'const A: &str = "save";\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("白名单条目已无生产 CJK src-tauri/src/lib.rs");
  });

  // ② 同一份文件写进例外表 → 放行。例外表是唯一的出口：没有它就只能删条目，
  //    而删掉会让规则 2 把文件里剩下的中文报出来（那是另一回事，不该由棘轮逼着做）。
  it("② 同一文件写进例外表 → 退出 0", () => {
    const { code, stdout } = runGate(
      { "src-tauri/src/lib.rs": 'const A: &str = "save";\n' },
      { rustPendingExceptions: [{ path: "src-tauri/src/lib.rs", residual: 0 }] }
    );
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // ③ 仍有生产 CJK 的条目：不报。这一条是**反面**——棘轮不能宽到把所有条目都报出来，
  //    否则各批的正常迁移会被它挡住，而闸门一旦乱叫就会被调松。
  it("③ 白名单条目的文件仍有生产 CJK → 不报（基线夹具）", () => {
    const { code, stdout } = runGate();
    expect(code).toBe(0);
    expect(stdout).not.toContain("白名单条目已无生产 CJK");
  });

  // ④ 例外表也是规则 2 的放行名单（与 RUST_UNTRANSLATED 同一处放行）：文件可以**移出白名单**
  //    而它的中文不被报成硬编码。少了这一半，`session_index/tests.rs` 这类纯测试文件就只能
  //    永远占着白名单——条目删不掉（规则 2 会报它 31 条测试中文），白名单也就无法诚实地清零。
  //    夹具模拟的正是那个终态：文件不在 RUST_PENDING 里，只在例外表里，且带生产 CJK。
  it("④ 例外表里的文件不在白名单也不被规则 2 报出", () => {
    const { code, stdout } = runGate(
      { "src-tauri/src/tests_only.rs": 'const A: &str = "测试夹具";\n' },
      { rustPendingExceptions: [{ path: "src-tauri/src/tests_only.rs", residual: 1 }] }
    );
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // 反面：同一份文件**不**写进例外表时规则 2 照旧报出——证明 ④ 的放行确实来自例外表，
  // 而不是「闸门根本没读这个文件」。少了这条对照，④ 在「扫描根漏了这个文件」时也会通过。
  it("④ 的对照：同一文件不在例外表里时规则 2 报出", () => {
    const { code, stderr } = runGate({ "src-tauri/src/tests_only.rs": 'const A: &str = "测试夹具";\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("硬编码中文 src-tauri/src/tests_only.rs: 测试夹具");
  });

  // ⑤ 条目指向一个**不存在**的文件（路径写错、文件被改名或删除）：报「不存在」，
  //    而不是「中文已迁完」——两种失效的原因与修法都不同（一个改路径/删条目，一个删条目），
  //    合成一条会把读者引向错误的方向。
  it("⑤ 白名单条目指向不存在的文件 → 报「不存在」，不报「已无生产 CJK」", () => {
    const { code, stderr } = runGate({}, { rustPending: ["src-tauri/src/ghost.rs"] });
    expect(code).toBe(1);
    expect(stderr).toContain("白名单条目指向的文件不存在（或不是文件）src-tauri/src/ghost.rs");
    expect(stderr).not.toContain("已无生产 CJK");
  });

  // ⑥ ★ 例外表的条目必须是一个**存在的文件**。这条守卫挡两种条目：
  //    ① 目录条目——例外表按整条路径**精确比对**（isExempted），所以目录条目谁也豁免不了，
  //       但把它留在表里会让下一个读者以为「那个目录已被豁免」；前缀匹配下它更会同时遮蔽
  //       整棵子树（规则 2 与棘轮一起失效）。少了下面那条守卫，这类条目不是被静默放行，而是
  //       让整轮诊断崩掉：棘轮的 readFileSync 对目录抛未捕获的 EISDIR、对不存在的路径抛
  //       ENOENT，都 exit 1 且打不出一条失败信息——守卫把它换成一条可读的失败，用例 ⑥ 钉的
  //       就是这个转换。
  //    ② 失效条目——写错路径、文件被改名或删除。本表**有**残量棘轮，但棘轮对这一类条目
  //       无能为力：它的判据是「条目里声称的残量与实测相符」，接不住一条写错的路径（它只会
  //       对着不存在的路径抛异常）。接住它们的是下面那条守卫，故守卫就是失效条目的
  //       「条目仍然必要」判据——而 T2–T10 九批都会往里写条目。
  it("⑥ 例外表里的目录条目与失效条目都报出", () => {
    const dir = runGate({}, { rustPendingExceptions: [{ path: "src-tauri/src", residual: 0 }] });
    expect(dir.code).toBe(1);
    expect(dir.stderr).toContain("例外表条目不是已存在的文件 src-tauri/src");

    const ghost = runGate({}, { rustPendingExceptions: [{ path: "src-tauri/src/ghost.rs", residual: 0 }] });
    expect(ghost.code).toBe(1);
    expect(ghost.stderr).toContain("例外表条目不是已存在的文件 src-tauri/src/ghost.rs");
  });

  // ⑥ 的另一半：例外表**不做前缀匹配**——命中只认整条路径相等，一条「长得像前缀」的条目
  // 不能顺带豁免它下面的文件。夹具刻意让条目是文件路径的**真前缀**（`…/tests_only` 之于
  // `…/tests_only.rs`）：前缀匹配下它会命中，而那条路径并不存在。
  // 两半都要钉：规则 2 那一半遮蔽的是「子树里新写的中文」，棘轮那一半遮蔽的是
  // 「条目已经没必要了」——两者都是本文件反复记过的静默失明。
  it("⑥ 的对照：例外表的前缀不命中其下的文件（规则 2 那一半）", () => {
    const { code, stderr } = runGate(
      { "src-tauri/src/tests_only.rs": 'const A: &str = "测试夹具";\n' },
      { rustPendingExceptions: [{ path: "src-tauri/src/tests_only", residual: 0 }] }
    );
    expect(code).toBe(1);
    expect(stderr).toContain("例外表条目不是已存在的文件 src-tauri/src/tests_only");
    // 前缀若命中，这一条就会消失：闸门看起来守住了，而其实放行了整棵子树。
    expect(stderr).toContain("硬编码中文 src-tauri/src/tests_only.rs: 测试夹具");
  });

  it("⑥ 的对照：例外表的前缀不豁免白名单条目（棘轮那一半）", () => {
    const { code, stderr } = runGate(
      { "src-tauri/src/lib.rs": 'const A: &str = "save";\n' },
      { rustPendingExceptions: [{ path: "src-tauri/src/lib", residual: 0 }] }
    );
    expect(code).toBe(1);
    expect(stderr).toContain("例外表条目不是已存在的文件 src-tauri/src/lib");
    // 前缀若命中，零 CJK 的条目会被静默豁免——棘轮再也报不出「这条已经没必要了」。
    expect(stderr).toContain("白名单条目已无生产 CJK src-tauri/src/lib.rs");
  });
});

// 例外表条目里写的残量（`residual`）必须等于该文件**实测**的生产 CJK 数。
// 少了这条，条目里的数字只是散文：文件再长出中文、或残留被迁走，闸门都不会响，
// 过期条目只能靠人重测才浮得出来（计划 10 已付过两次这个代价）。
describe("例外表棘轮", () => {
  // 夹具文件里有 1 条生产 CJK，但条目声称 2 条 —— 棘轮必须报出来。
  const FIXTURE_FILE = 'pub fn f() {\n    let _ = "中文";\n}\n';

  it("声称的残量与实测不符时闸门失败", () => {
    const { code, stderr } = runGate(
      { "src-tauri/src/whatever.rs": FIXTURE_FILE },
      { rustPendingExceptions: [{ path: "src-tauri/src/whatever.rs", residual: 2 }] },
    );
    expect(code).toBe(1);
    expect(stderr).toContain(
      "例外表棘轮：src-tauri/src/whatever.rs 声称残 2 条，实测 1 条（差 -1）",
    );
  });

  // 阳性对照：没有它，「恒报错」与「真的只在不符时报」在输出上不可区分。
  it("阳性对照：声称与实测相符时不报", () => {
    const { code, stderr } = runGate(
      { "src-tauri/src/whatever.rs": FIXTURE_FILE },
      { rustPendingExceptions: [{ path: "src-tauri/src/whatever.rs", residual: 1 }] },
    );
    expect(code).toBe(0);
    expect(stderr).not.toContain("例外表棘轮");
  });
});

// 剥注释失明是静默的，所以要有一道自检把它变成响亮的失败：剥完之后花括号不平衡、或字符串
// 一直没闭合，就说明剥注释结果不可信，此时必须在错误的输入上停下，而不是继续拿它跑规则。
// 判据刻意不指向任何单一原因，故各钉一种后果。
describe("check-i18n.mjs 剥注释自检", () => {
  it("花括号不平衡时退出非 0 并报出原因", () => {
    const { code, stderr } = runGate({ "src/app.ts": "const a = 1;\n}\n" });
    expect(code).toBe(1);
    expect(stderr).toContain("剥注释结果不可信 src/app.ts: 代码位置的花括号不平衡（净 -1）");
  });

  // .vue 是按整份文件剥的（规则 1/5 都这么用），脚本块里一处未闭合的模板字面量就够触发。
  it("字符串未闭合时退出非 0 并报出原因", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": '<script setup lang="ts">\nconst s = `abc\n</script>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("剥注释结果不可信 src/Comp.vue: 扫描结束时仍在字符串字面量里");
  });

  // 自检与白名单无关：失明发生在剥注释这一步，文件迁没迁移都一样，跳过它等于放行一份
  // 「看起来守住了、其实一个字都没扫」的结果。
  it("白名单里的文件剥注释错位同样报出", () => {
    const { code, stderr } = runGate({
      [PENDING_FILE]: '<script setup lang="ts">\nconst s = `abc\n</script>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain(`剥注释结果不可信 ${PENDING_FILE}: 扫描结束时仍在字符串字面量里`);
  });

  // 合法模板正文里的撇号与落单花括号是对**正确代码**的假阳性，必须放行；脚本区的真实错位仍要
  // 响亮失败。只放前者会把「响亮地报错」换成「脚本区被静默失明」，比误报更糟，故两个方向都钉住。
  it("模板正文里的撇号与落单花括号放行，脚本区的真实错位照旧失败", () => {
    const ok = runGate({
      "src/Comp.vue": "<template><p>Don't save</p><!-- don't render this --></template>\n",
    });
    expect(ok.code).toBe(0);
    expect(ok.stdout).toContain("i18n 闸门通过");

    // 模板正文里的落单花括号同属这一类（`50} off` 这种英文文案正在进来）：按 JS 剥会在
    // 一份完全合法的文件上报「花括号不平衡」，净 +1 与净 -1 两个方向都要放行。
    for (const text of ["<template><p>{ use</p></template>\n", "<template><p>50} off</p></template>\n"]) {
      const brace = runGate({ "src/Comp.vue": text });
      expect(brace.code, text).toBe(0);
    }

    const bad = runGate({
      "src/Comp.vue":
        '<template><p>Don\'t save</p></template>\n<script setup lang="ts">\nconst a = 1;\n}\n</script>\n',
    });
    expect(bad.code).toBe(1);
    expect(bad.stderr).toContain("剥注释结果不可信 src/Comp.vue: 代码位置的花括号不平衡（净 -1）");
  });

  // 同行规则把「引号一路吞到文件末尾」这一位换掉了，脚本区的裸引号必须仍然响亮失败：
  // 旧实现在这两个夹具上都 exit 1，同行规则接管后若不再报，规则 1 会静默看不见其中的中文，
  // 规则 5 的引用会被掩掉而无人出声——正是本文件反复修的那类「响亮变静默」。
  it("脚本区里同行未闭合的引号退出非 0，不再静默放行", () => {
    const bare = runGate({ "src/app.ts": "const a = '你好;\nconst b = 1;\n" });
    expect(bare.code).toBe(1);
    expect(bare.stderr).toContain("剥注释结果不可信 src/app.ts: 单/双引号同行没有收尾引号（共 1 处）");

    // 正则被误判成除号（`)` 不在正则前置字符集合里），其中的引号与同一行后文的引号配了对，
    // 规则 1/5 的扫描确实被掩掉——但闸门不能一声不响。
    const regex = runGate({ "src/app.ts": "if (ok) /'/; const a = t(\"common.a\"); const b = 'x';\n" });
    expect(regex.code).toBe(1);
    expect(regex.stderr).toContain("剥注释结果不可信 src/app.ts: 单/双引号同行没有收尾引号（共 1 处）");
  });

  // 合法 JS 不该被这一位打扰：同行闭合的引号、续行、一元加号接正则。
  it("合法脚本区不被自检打扰", () => {
    expect(runGate({ "src/app.ts": 'const a = \'save\';\nconst b = "don\'t";\n' }).code).toBe(0);
    expect(runGate({ "src/app.ts": "const s = 'a\\\r\n}'; const b = 1;\r\n" }).code).toBe(0);
    expect(runGate({ "src/app.ts": "const ok = a + +/{/.test(s);\nconst b = 1;\n" }).code).toBe(0);
  });

  // 把自检收窄到脚本块会漏掉唯一一类跨区错位：模板正文里**落单**的反引号与脚本区的反引号
  // 配了对，两区之间的真实代码（含整段脚本）被当成一个字面量——块自己是对的，块内自查看不出来。
  // 旧实现靠整份文件扫一遍、引号吞到文件末尾把它响亮报出，故这一位必须补上。
  // 报出的措辞描述的是判据本身（非脚本区有落单的反引号），不断言「与脚本区配了对」：
  // 脚本区一个反引号都没有时同样会报，那时把责任推给脚本区会把读者引到错误的地方。
  it("脚本区之外有落单的反引号时退出非 0", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": '<template><p>a `b</p></template>\n<script setup lang="ts">\nconst s = `x`;\n</script>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("剥注释结果不可信 src/Comp.vue: 脚本区之外有落单的反引号");
  });

  // 模板正文里的反引号成对时是正文（帮助文案里的行内代码），不报。
  it("模板区自洽的反引号不报", () => {
    const { code } = runGate({
      "src/Comp.vue": '<template><p>a `b` c</p></template>\n<script setup lang="ts">\nconst s = `x`;\n</script>\n',
    });
    expect(code).toBe(0);
  });

  // 已知代价（刻意保留，理由见 templateRegionSwallowsScript 的说明）：判据只看非脚本区有没有
  // 落单的反引号，不看脚本区有没有反引号可配对，故没有 script 块时同样会报——一份合法文件上的
  // 响亮失败。真实树上 0 处，且要同时满足「模板正文有**落单**的反引号」与「脚本区反引号数为奇」
  // 两个少见形态。不换成更聪明的启发式：它守的是整段脚本静默不被扫描那一类，方向不能反过来。
  it("脚本区没有反引号时同样报出（已知代价）", () => {
    const { code, stderr } = runGate({ "src/Comp.vue": "<template><p>a `b</p></template>\n" });
    expect(code).toBe(1);
    expect(stderr).toContain("剥注释结果不可信 src/Comp.vue: 脚本区之外有落单的反引号");
  });

  // 干净夹具照旧放行：自检是守卫，不是新的误报源。
  it("剥注释可信时照常退出 0", () => {
    const { code } = runGate();
    expect(code).toBe(0);
  });

  // 模板正文不是 JS，而 templateRegionSwallowsScript 会把整份文件（含模板正文）交给剥注释扫描，
  // 故扫描器的 `\` 分支真的会被走到：Windows 路径 `C:\Users\x` 在闸门真实路径上会让它走到 6 次
  // （分解见 scanJsComments 里的实测）。
  // 这条只钉「走到也不产生误判」——它断言 exit 0，分不开「走到了但没误判」与「根本没走到」；
  // 可达性由 stripJsComments 那一组里的「`\` 之后的 `/*` 不开启块注释」钉住，两者合起来才完整。
  it("模板正文里的 Windows 路径不误报（本用例不钉可达性）", () => {
    const { code } = runGate({
      "src/Comp.vue": '<template>\n  <p>C:\\Users\\x</p>\n</template>\n<script setup lang="ts">\nconst a = 1;\n</script>\n',
    });
    expect(code).toBe(0);
  });
});

// 规则 5 守的是「用 t() 写的错别字会原样渲染到界面」：missingWarn 关掉后 vue-i18n 返回 key
// 本身，写错的 key 不会报错、只会显示给用户。夹具的 en 包只有 common.a，common.b 是现成的缺失 key。
// 断言 exit 0 的夹具把 t() 写在箭头函数里：模块顶层的 `const b = t(…)` 会被规则 7 报成
// 只求值一次，那是规则 7 的用例，不该混进规则 5 的用例。
describe("check-i18n.mjs 规则 5：引用的 key 必须存在", () => {
  it("引用不存在的 key 时退出非 0 并报出文件、行号与 key", () => {
    const { code, stderr } = runGate({ "src/app.ts": 'const a = "save";\nconst b = t("common.b");\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("引用的翻译 key 不存在 src/app.ts:2: common.b");
  });

  it("引用存在的 key 时放行", () => {
    const { code } = runGate({ "src/app.ts": 'const b = () => t("common.a");\n' });
    expect(code).toBe(0);
  });

  it("引用指向子键集合的 key 时退出非 0", () => {
    const { code, stderr } = runGate({ "src/app.ts": 'const b = t("common");\n' });
    expect(code).toBe(1);
    expect(stderr).toContain("引用的翻译 key 指向子键集合 src/app.ts:1: common");
  });

  it("Vue 模板表达式里的 t() 同样受约束", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": '<template><div :title="t(\'common.b\')">save</div></template>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("引用的翻译 key 不存在 src/Comp.vue:1: common.b");
  });

  // 与既有规则一致：注释里的 t( 不是引用。块注释还必须按行占位，否则其后所有行号前移，
  // CI 里给出的行号会指向错误的位置。
  it("注释里的 t() 不算引用，且块注释不让行号前移", () => {
    const { code, stderr } = runGate({
      "src/app.ts": '/*\n\nt("common.b")\n*/\n// t("common.b")\nconst b = t("common.b");\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("引用的翻译 key 不存在 src/app.ts:6: common.b");
  });

  it("字符串拼接的实参不当作 key 引用", () => {
    const { code } = runGate({ "src/app.ts": 'const b = () => t("common." + suffix);\n' });
    expect(code).toBe(0);
  });

  // 字符串里的 `/*` 曾让剥注释把其后真实代码一路抹到下一个 `*/`：落在里面的 t() 引用
  // 对规则 5 完全不可见，闸门照常报「通过」。仓库里 svg.ts 与 i18n/index.ts 各有一处。
  it("字符串里的 /* 之后的 t() 引用不再被静默跳过", () => {
    const { code, stderr } = runGate({
      "src/app.ts": 'const a = "/* not a comment";\nconst b = t("common.b");\n/* 真注释 */\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("引用的翻译 key 不存在 src/app.ts:2: common.b");
  });

  // 正则字面量里的引号曾让引号配对错位，**该文件其余部分再也不被扫描**：src/utils/json.ts
  it("动态 key 的静态前缀存在时放行", () => {
    const { code } = runGate({ "src/app.ts": "const b = () => t(`common.a${x}`);\n" });
    expect(code).toBe(0);
  });

  it("动态 key 的静态前缀不存在时退出非 0", () => {
    const { code, stderr } = runGate({ "src/app.ts": "const b = t(`settings.nav.${x}`);\n" });
    expect(code).toBe(1);
    expect(stderr).toContain("动态 key 前缀不存在 src/app.ts:1: settings.nav");
  });

  // 前缀落在叶子上是假通过：`common.a${x}` 拼出来的是 common.a 开头的另一个键，不是 common.a。
  it("动态 key 的前缀落在叶子上时退出非 0", () => {
    const { code, stderr } = runGate({ "src/app.ts": "const b = t(`common.a.${x}`);\n" });
    expect(code).toBe(1);
    expect(stderr).toContain("动态 key 前缀指向文本 src/app.ts:1: common.a");
  });

  // 静态前缀截到最后一个点，而不是取到最后一个完整键名：相邻键 common.a 与 common.ab 并存时，
  // 把 `common.a${x}` 当成「叶子 common.a 后面接了东西」会误报。
  it("动态 key 的静态前缀截到最后一个点，相邻键不被误判", () => {
    const { code } = runGate({
      // 四份语言包同时补 ab：只改 en 会撞上规则 3 的语言包缺口，用例就不再是规则 5 的用例了。
      "src/locales/en/common.json": '{ "a": "1", "ab": "2" }\n',
      "src/locales/zh/common.json": '{ "a": "x", "ab": "y" }\n',
      "src/locales/ja/common.json": '{ "a": "x", "ab": "y" }\n',
      "src/locales/de/common.json": '{ "a": "x", "ab": "y" }\n',
      "src/app.ts": "const b = () => t(`common.a${x}`);\n",
    });
    expect(code).toBe(0);
  });

  it("取不出静态前缀的动态 key 跳过", () => {
    const { code } = runGate({ "src/app.ts": "const b = () => t(`${ns}.title`);\n" });
    expect(code).toBe(0);
  });

  // 错别字最典型的藏身处就是「组件与断言写同一个错 key」——两边都拿到 key 路径，断言照常通过。
  // 测试文件因此在规则 5 的扫描范围内（规则 1 排除 .test.ts 是因为夹具里本就有中文样例数据，
  // 与「key 是否存在」无关）。
  it("测试文件里的 t() 引用同样受约束", () => {
    const { code, stderr } = runGate({
      "src/components/Foo.test.ts": 'expect(rendered).toBe(t("common.b"));\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("引用的翻译 key 不存在 src/components/Foo.test.ts:1: common.b");
  });

  // 回退链测试需要一个四份语言包都没有的 key，它是测试装置而非笔误，按名单豁免；
  // 豁免必须窄到只覆盖这一个 key——同文件里其它缺失 key 仍要报出来。
  it("刻意引用不存在 key 的探针放行，探针之外的缺失 key 照旧报出", () => {
    const { code, stderr } = runGate({
      "src/i18n/index.test.ts": 'const a = t("__nonexistent__.key");\nconst b = t("__other__.key");\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("__other__.key");
    expect(stderr).not.toContain("__nonexistent__.key");
  });
});

// 规则 7 守的是「只求值一次的 t()」：语言包文案必须在渲染时求值，否则在那一刻
// 被冻结成当时的语言，用户之后切换语言这些文案不动。这一类在每个迁移任务里都被手工漏过，
// 闸门是唯一能挡住它回归的地方。夹具一律用 common.a——它存在，规则 5 不会跟着报。
describe("check-i18n.mjs 规则 7：只求值一次的 t()", () => {
  it("模块级选项数组退出非 0，报出文件、行号与声明原文", () => {
    const { code, stderr } = runGate({
      "src/opts.ts": 'const OPTS = [{ label: t("common.a") }, { label: t("common.a") }];\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain(
      '模块加载/组件挂载时求值一次的 t() src/opts.ts:1: const OPTS = [{ label: t("common.a") }, { label: t("common.a") }];'
    );
  });

  // .vue 的声明在 script setup 块里，行号必须折算成文件里的行号。模板排在前面、块首在第 5 行，
  // 声明在块内第 4 行 → 文件第 8 行；不折算（直接用块内行号）会报成第 4 行。
  it("script setup 块里的模块级声明退出非 0，行号按文件折算", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue":
        '<template>\n  <div>save</div>\n</template>\n\n<script setup lang="ts">\nimport { t } from "./i18n";\n\nconst OPTS = [{ label: t("common.a") }];\n</script>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain('模块加载/组件挂载时求值一次的 t() src/Comp.vue:8: const OPTS = [{ label: t("common.a") }];');
  });

  // 段尾是 `}` 的语句（`export interface` / `function`）不以 `;` 收尾，紧随其后的模块级常量
  // 曾经被并进一个段首是关键字的长段而整段丢弃——ProfileImportConflictDialog.vue 的两处
  // 选项常量就排在 `export interface` 块之后。
  it("export interface 之后的模块级常量退出非 0", () => {
    const { code, stderr } = runGate({
      "src/opts.ts":
        'export interface Option {\n  label: string;\n}\n\nconst OPTS: Option[] = [{ label: t("common.a") }];\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("模块加载/组件挂载时求值一次的 t() src/opts.ts:5:");
  });

  // 未赋值的宏调用同样在加载期求值（仓库里真有这种写法），故这条必须承重。
  it("未赋值的 withDefaults 调用退出非 0", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue":
        '<script setup lang="ts">\nwithDefaults(defineProps<{ label?: string }>(), { label: t("common.a") });\n</script>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("模块加载/组件挂载时求值一次的 t() src/Comp.vue:2: withDefaults(");
  });

  it("computed 里求值的 t() 放行", () => {
    const { code } = runGate({
      "src/opts.ts": 'const OPTS = computed(() => [{ label: t("common.a") }]);\n',
    });
    expect(code).toBe(0);
  });

  it("函数体内的 t() 放行", () => {
    const { code } = runGate({ "src/opts.ts": 'function label() {\n  return t("common.a");\n}\n' });
    expect(code).toBe(0);
  });

  // 模板不是模块作用域：模板文本与插值里的引用由规则 1/5 管，规则 7 只扫 script 块。
  it("模板里的 t() 不由本规则报出", () => {
    const { code } = runGate({
      "src/Comp.vue":
        '<script setup lang="ts">\nimport { t } from "./i18n";\n</script>\n<template><div :title="t(\'common.a\')">save</div></template>\n',
    });
    expect(code).toBe(0);
  });

  // 与其它规则一致：白名单里的文件是「已知待迁移」，其文案问题随各自任务处理；
  // 文件一旦移出白名单，本规则即开始保护它。
  it("白名单里的模块级 t() 放行", () => {
    const { code } = runGate({
      [PENDING_FILE]: 'export const OPTS = [{ label: t("common.a") }];\n',
    });
    expect(code).toBe(0);
  });

  // 跳过的文件数必须出现在 CI 日志里（与规则 1 同一份白名单、同一个理由）。
  it("非白名单文件里的模块级 t() 与其它违规同时报出，互不吞没", () => {
    const { code, stderr } = runGate({
      "src/opts.ts": 'const OPTS = [{ label: t("common.a") }];\n',
      "src/other.ts": 'const a = "保存";\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("模块加载/组件挂载时求值一次的 t() src/opts.ts:1:");
    expect(stderr).toContain("硬编码中文 src/other.ts: 保存");
  });

  // 缺陷①在闸门这一层的形状：空体多行箭头是合法代码，旧实现会在这份文件上 exit 1
  // （报出第 2 行那个 `t(…)`）。本仓以 `=>` 结尾的行有 74 行，这不是假想写法。
  it("空体多行箭头不报，闸门在合法代码上退出 0", () => {
    const { code, stdout } = runGate({
      "src/opts.ts": 'const formatRowLabel = (row: { name: string }) =>\n  t("common.a");\n',
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // 缺陷②在闸门这一层的形状：无分号风格里，前一条语句不是候选段首时整段被丢弃。
  // 两个真实文件（useStreamingLoad.ts / useStreamingCollection.ts，批次 5 的目标）整份不写分号。
  it("无分号语句之后紧跟的模块级 t() 退出非 0", () => {
    const { code, stderr } = runGate({
      "src/opts.ts": 'const A = 1\narr.push(t("common.a"))\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain('模块加载/组件挂载时求值一次的 t() src/opts.ts:2: arr.push(t("common.a"))');
  });

  // 解析失败必须响亮：静默跳过是本文件全部六次缺陷的共同形状，故这里既报出文件与位置，
  // 也说明「该文件的加载期 t() 未被检查」——不能让它看起来像一次通过。
  it("脚本解析失败时退出非 0，报出文件与位置而不是跳过", () => {
    const { code, stderr } = runGate({
      "src/opts.ts": 'const OPTS = t("common.a");\n}\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("规则 7 无法解析 src/opts.ts:2:1：Declaration or statement expected.");
    expect(stderr).toContain("（该文件的加载期 t() 未被检查）");
  });

  // 行号必须折算成**文件**行号：`.vue` 的脚本块不在文件开头，报块内行号会把读者指到模板上。
  // 夹具里脚本块从第 4 行开始，错误在块内第 3 行（`}` 那行）→ 文件第 6 行。
  it("script 块里的解析失败按文件行号报出，不报块内行号", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue":
        '<template>\n  <div>save</div>\n</template>\n<script setup lang="ts">\nconst OPTS = t("common.a");\n}\n</script>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("规则 7 无法解析 src/Comp.vue:6:1：");
    expect(stderr).not.toContain("规则 7 无法解析 src/Comp.vue:3");
  });

  // 解析失败不能中断整轮：main() 逐块 catch，同一轮里其它文件的违规照常报出。
  it("解析失败与其它文件的违规同时报出，互不吞没", () => {
    const { code, stderr } = runGate({
      "src/broken.ts": 'const OPTS = t("common.a");\n}\n',
      "src/other.ts": 'const a = "保存";\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("规则 7 无法解析 src/broken.ts:2:1：");
    expect(stderr).toContain("硬编码中文 src/other.ts: 保存");
  });
});

// 规则 8 守的是**新增**的英文界面文案：规则 1 只认 CJK 码点，一句新写的 `Save changes`
// 对闸门完全不可见，而这个类已经漏过两次（计划 2 与计划 5 的最终评审）。
// 判据（模板文本节点里含 ASCII 字母）会命中大量非文案，噪声由**允许清单**消化——
// 清单不是豁免表，是从 scripts/i18n-english-adjudication.json 派生的裁决集（见闸门里的说明）。
describe("check-i18n.mjs 规则 8：模板文本节点里的新增英文文案", () => {
  it("不在允许清单里的英文文案退出非 0 并点名文件与那串", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": "<template><div>Save changes</div></template>\n",
    });
    expect(code).toBe(1);
    expect(stderr).toContain("新增英文界面文案（模板文本/静态属性）src/Comp.vue: Save changes");
  });

  it("同一串进了夹具的允许清单就放行", () => {
    const { code, stdout } = runGate({
      "src/Comp.vue": "<template><div>Save changes</div></template>\n",
      "scripts/i18n-english-allowlist.json": fixtureAllowlist({ "Save changes": "夹具里已裁定" }),
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // 判据只管「含 ASCII 字母」：`{{ count }}` 在 AST 里是 INTERPOLATION、根本不是文本节点，
  // `×` 一个字母都没有——两者都不该报。这一条钉住判据没有宽到把插值与符号卷进来。
  it("纯插值与纯符号不报", () => {
    expect(findUnlistedEnglishTexts("<template><div>{{ count }}</div><div>×</div></template>", {})).toEqual([]);
    const { code } = runGate({ "src/Comp.vue": "<template><div>{{ count }}</div><div>×</div></template>\n" });
    expect(code).toBe(0);
  });

  // 属性这一半是计划 6 的评审补的：只覆盖文本节点时，`title="Save changes"` 这类
  // **同样是新写的英文界面文案**仍然无声通过（实测今天就有 4 个不同串）。
  // 窄口径只取 title / aria-label / placeholder / alt。**实测宽窄的差距**（T6 收口时复跑，
  // 口径写在括号里）：今天 `.vue` 里共有 3994 个静态属性值，窄口径命中 4 条 / 3 个不同串
  // （全部已在允许清单里，故新增条目 0）；宽口径若照闸门规则 8 的判据（含字母）命中
  // 3706 条 / 1959 个不同串，其中 **1955 个不在允许清单里**——即便改用
  // `isEnglishCandidate` 的形状过滤（对宽口径最有利的算法），仍有 285 条 / 206 个不同串、
  // **202 个不在清单里**。宽口径的清单会因此无界增长，而那些位置（class/style/id/data-*）
  // 的值按构造不是文案，所以取窄口径。
  it("四个静态文本属性位置都被覆盖", () => {
    for (const attr of ["title", "aria-label", "placeholder", "alt"]) {
      expect(findUnlistedEnglishTexts(`<template><input ${attr}="Save changes" /></template>`, {})).toEqual([
        "Save changes",
      ]);
    }
  });

  it("静态文本属性里的新增英文文案退出非 0 并点名文件与那串", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": '<template><input title="Save changes" /></template>\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("新增英文界面文案（模板文本/静态属性）src/Comp.vue: Save changes");
  });

  it("静态文本属性里的串进了允许清单就放行", () => {
    const { code, stdout } = runGate({
      "src/Comp.vue": '<template><input placeholder="Save changes" /></template>\n',
      "scripts/i18n-english-allowlist.json": fixtureAllowlist({ "Save changes": "夹具里已裁定" }),
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // 窄口径的守卫：其余属性的值不是文案，动态绑定（`:title="…"`，AST 里是 DIRECTIVE 不是
  // 静态属性）的值是表达式。两者都不该报——判据一旦宽到这里，允许清单会随每次新增
  // 类名/标识符无界增长，而那正是这份清单在设计上要避免的东西。
  it("非文本属性与动态绑定不报", () => {
    const src =
      '<template><input class="save changes" id="save-changes" data-label="Save changes" :title="saveChanges" /></template>';
    expect(findUnlistedEnglishTexts(src, {})).toEqual([]);
    const { code } = runGate({ "src/Comp.vue": src + "\n" });
    expect(code).toBe(0);
  });

  // 判定按**整个文本节点**（trim 后），不按空白切词：允许清单是裁决集 `text` 的投影，
  // 而裁决集记的就是整个文本节点。按词切会让整句条目在清单里落空，把已裁定过的重新报出来。
  it("按整个文本节点比对，不按空白拆词", () => {
    const src = "<template><div>history tab is missing a CLI identity</div></template>";
    expect(findUnlistedEnglishTexts(src, { "history tab is missing a CLI identity": "已裁定" })).toEqual([]);
    expect(findUnlistedEnglishTexts(src, { history: "只给了其中一个词" })).toEqual([
      "history tab is missing a CLI identity",
    ]);
  });

  // 清单条目必须带理由：它是裁决集的投影，没有理由的条目就是一条无名豁免，
  // 而「无名豁免」正是这份清单在设计上要避免的东西。
  it("允许清单里没有理由的条目退出非 0", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": "<template><div>Save changes</div></template>\n",
      "scripts/i18n-english-allowlist.json": '{\n  "Save changes": ""\n}\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("规则 8 允许清单缺理由（Save changes）");
  });

  // 模板解析不出来时不能中断整轮：跳过等于这份文件的模板文本无人检查，而闸门照常报「通过」。
  it("模板解析失败时报出该文件，与其它文件的违规互不吞没", () => {
    const { code, stderr } = runGate({
      "src/Broken.vue": "<template><div>Save changes</div></template>\n<template><div>again</div></template>\n",
      "src/other.ts": 'const a = "保存";\n',
    });
    expect(code).toBe(1);
    expect(stderr).toContain("规则 8 无法解析 src/Broken.vue：");
    expect(stderr).toContain("（该文件的模板文本与静态属性未被检查）");
    expect(stderr).toContain("硬编码中文 src/other.ts: 保存");
  });

  // 清单读不到或读不成 JSON 时必须**硬失败**，不许退化成「按空表继续跑规则」：
  // 空表意味着每一条已经裁定过的条目都会被当成新增文案报出来（闸门乱叫会被调松），
  // 更糟的是它看起来像一次正常运行。这两条用例断言的不是「退出非 0」本身——
  // 退化成空表同样会退出非 0（下面的 .vue 里有一句不在清单里的英文）——而是
  // **闸门没有继续跑规则**：既没报「通过」，也没把那句英文报成新增文案。
  // 变异实测：把 loadEnglishAllowlist 改成 try/catch 回退 `{}`，两条用例都会变红。
  it("允许清单缺失时硬失败，不静默按空表继续跑规则", () => {
    const root = buildFixture({ "src/Comp.vue": "<template><div>Save changes</div></template>\n" });
    rmSync(join(root, "scripts/i18n-english-allowlist.json"));
    const { code, stderr } = runScript(join(root, "scripts/check-i18n.mjs"));
    expect(code).not.toBe(0);
    expect(stderr).toContain("i18n-english-allowlist.json");
    expect(stderr).not.toContain("i18n 闸门通过");
    expect(stderr).not.toContain("新增英文界面文案");
  });

  it("允许清单不是合法 JSON 时硬失败，不静默按空表继续跑规则", () => {
    const { code, stderr } = runGate({
      "src/Comp.vue": "<template><div>Save changes</div></template>\n",
      "scripts/i18n-english-allowlist.json": '{ "save": "夹具基线"  <<坏掉的 JSON\n',
    });
    expect(code).not.toBe(0);
    expect(stderr).toContain("SyntaxError");
    expect(stderr).not.toContain("i18n 闸门通过");
    expect(stderr).not.toContain("新增英文界面文案");
  });
});

/**
 * 规则 9 夹具的四份 `common.json`：键集合四语一致（规则 3 要求），
 * 差异只在取值——用例要构造的正是「同一 `en`、`ja`/`de` 取值不同」。
 */
function fixturePacks({ en, zh, ja, de }) {
  const file = (o) => `${JSON.stringify(o, null, 2)}\n`;
  return {
    "src/locales/en/common.json": file(en),
    "src/locales/zh/common.json": file(zh),
    "src/locales/ja/common.json": file(ja),
    "src/locales/de/common.json": file(de),
  };
}

/** 夹具漂移允许清单的 JSON 文本。 */
function fixtureDriftAllowlist(entries = {}) {
  return `${JSON.stringify(entries, null, 2)}\n`;
}

// 规则 9 的存在理由：Task 5/6 **修好了现状**（合并、统一、拆身份），但**没有阻止复发**——
// 将来任何人在任何命名空间里新建一个与既有键同 `en`、`ja`/`de` 却不同的键，闸门此前看不见。
// 本仓已为此付过代价：同一个 `Close` 在计划 4 内被两个任务答成了相反方向。
//
// 判据按 Ruling R3，**不是 spec 的原话**：spec 写的是「同一 `en` 值、目标语言取值不同即报警」，
// 而该前提在计划 7 开工前就已被证伪——**43 组漂移全部是合法的 `keep`**（①A/①B 两条不同源文案），
// 照原话写会在第一天就变红。故判据是「同 `en`、`ja`/`de` 分歧、**且不在裁决集里**」。
describe("check-i18n.mjs 规则 9：同一 en 值下的 ja/de 译文漂移", () => {
  /** ① / ② 共用的夹具：同 `en`、同 `zh`、同 `ja`，只有 `de` 不同——纯译文漂移，无歧义。 */
  const DE_DRIFT = fixturePacks({
    en: { save: "Save changes", saveBtn: "Save changes" },
    zh: { save: "保存", saveBtn: "保存" },
    ja: { save: "保存する", saveBtn: "保存する" },
    de: { save: "Speichern", saveBtn: "Änderungen speichern" },
  });

  it("① 同一 en、de 不同且不在允许清单里 → 退出非 0，点名两个键与那个 en 值", () => {
    const { code, stderr } = runGate(DE_DRIFT);
    expect(code).toBe(1);
    expect(stderr).toContain("common.save");
    expect(stderr).toContain("common.saveBtn");
    expect(stderr).toContain("Save changes");
  });

  it("② 同一个夹具，把该 en 值加进夹具的允许清单 → 退出 0", () => {
    const { code, stdout } = runGate({
      ...DE_DRIFT,
      "scripts/i18n-drift-allowlist.json": fixtureDriftAllowlist({ "Save changes": "夹具里已裁定" }),
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // ③ 这一条承载 R3 的全部要点：`zh` 分歧**不参与**规则 9——它正是「两条不同源文案」的直接证据
  //    （①A/①B），而规则 9 只看 `ja`/`de`。
  //    夹具的 `zh` 逐字不同、`ja`/`de` 逐字相同，故判据一旦把 `zh` 也算进去，本条立刻变红。
  //    **若把它写松（例如让 ja/de 也不同），它就会因为一个与 R3 无关的原因通过。**
  it("③ 同一 en、ja/de 相同、只有 zh 不同 → 不报（①A/①B 的形态）", () => {
    const { code, stdout } = runGate(
      fixturePacks({
        en: { save: "Save changes", saveBtn: "Save changes" },
        zh: { save: "保存", saveBtn: "保存更改" },
        ja: { save: "保存する", saveBtn: "保存する" },
        de: { save: "Speichern", saveBtn: "Speichern" },
      }),
    );
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  it("④ 两个键 en 不同 → 不报", () => {
    const { code, stdout } = runGate(
      fixturePacks({
        en: { save: "Save changes", del: "Delete file" },
        zh: { save: "保存", del: "删除文件" },
        ja: { save: "保存する", del: "ファイルを削除" },
        de: { save: "Speichern", del: "Datei löschen" },
      }),
    );
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // 四种形态一次钉住。判据函数**只接受 en/ja/de**——`zh` 连参数位都没有，
  // 故「zh 分歧不算漂移」在函数签名上是结构性的，不靠调用方自觉。
  it("判据：ja 或 de 取值不唯一才算漂移，en 不同或目标语一致都不算", () => {
    const packs = {
      en: {
        "c.jaDrift": "A",
        "c.jaDrift2": "A",
        "c.deDrift": "B",
        "c.deDrift2": "B",
        "c.agree": "C",
        "c.agree2": "C",
        "c.solo": "D",
        "c.solo2": "E",
      },
      ja: {
        "c.jaDrift": "あ",
        "c.jaDrift2": "い",
        "c.deDrift": "う",
        "c.deDrift2": "う",
        "c.agree": "え",
        "c.agree2": "え",
        "c.solo": "お",
        "c.solo2": "か",
      },
      de: {
        "c.jaDrift": "X",
        "c.jaDrift2": "X",
        "c.deDrift": "Y",
        "c.deDrift2": "Z",
        "c.agree": "W",
        "c.agree2": "W",
        "c.solo": "V",
        "c.solo2": "U",
      },
    };
    const hits = findUnlistedDrift(packs, {});
    expect(hits.map((h) => h.en)).toEqual(["A", "B"]);
    expect(hits[0].keys).toEqual(["c.jaDrift", "c.jaDrift2"]);
    expect(hits[1].keys).toEqual(["c.deDrift", "c.deDrift2"]);
    expect(findUnlistedDrift(packs, { A: "已裁定" }).map((h) => h.en)).toEqual(["B"]);
  });

  // 与规则 8 同一道守卫：允许清单是裁决集的投影，每条必须写明理由。
  // 没有理由的条目就是一条**无名豁免**，而「无名豁免」正是这份清单在设计上要避免的东西。
  it("允许清单里没有理由的条目退出非 0", () => {
    const { code, stderr } = runGate({
      ...DE_DRIFT,
      "scripts/i18n-drift-allowlist.json": fixtureDriftAllowlist({ "Save changes": "" }),
    });
    expect(code).toBe(1);
    expect(stderr).toContain("规则 9 允许清单缺理由（Save changes）");
  });

  // 读不到清单时**硬失败**，不许退化成「按空表继续跑规则」：空表意味着每一条已经裁定过的
  // 漂移组都会被当成新增漂移报出来（闸门乱叫会被调松），更糟的是它看起来像一次正常运行。
  // 断言的不是「退出非 0」本身——退化成空表同样会退出非 0——而是**闸门没有继续跑规则**。
  it("允许清单缺失时硬失败，不静默按空表继续跑规则", () => {
    const root = buildFixture(DE_DRIFT);
    rmSync(join(root, "scripts/i18n-drift-allowlist.json"));
    const { code, stderr } = runScript(join(root, "scripts/check-i18n.mjs"));
    expect(code).not.toBe(0);
    expect(stderr).toContain("i18n-drift-allowlist.json");
    expect(stderr).not.toContain("i18n 闸门通过");
    expect(stderr).not.toContain("译文漂移");
  });
});

describe("findCodedCodeLiterals", () => {
  it("取出 coded() 的字面量 code 与文件行号", () => {
    expect(findCodedCodeLiterals('pub fn f() -> AppError {\n    AppError::coded("a.b")\n}\n')).toEqual([
      { code: "a.b", line: 2, keys: null },
    ]);
  });

  // 链式站点的 `with()` 键名：只有 `.with(...)` **直接链在 coded() 之后**（同一表达式）时才取得到。
  // 换行不算断开——本仓 `assistant/commands.rs:441-442`（行号在 `987b91a0` 上量得）就是这种跨行写法，
  // 判据若只认同一行，那处真实的 `{status}` 站点会被当成「没有 with()」，规则在那里就是空的。
  it("取出直接链在 coded() 之后的 with() 键名，跨行不断开", () => {
    expect(
      findCodedCodeLiterals('pub fn f() -> AppError {\n    AppError::coded("a.b").with("k", v)\n}\n')
    ).toEqual([{ code: "a.b", line: 2, keys: ["k"] }]);
    expect(
      findCodedCodeLiterals(
        'pub fn f() -> AppError {\n    AppError::coded("a.b")\n        .with("k", v)\n        .with("j", w)\n}\n'
      )
    ).toEqual([{ code: "a.b", line: 2, keys: ["k", "j"] }]);
  });

  // 判不出键名时返回 null，**不是**空数组：空数组的含义是「这个站点确实一个 with() 都没写」，
  // 而那是静态判不出来的——`with()` 可能挂在另一条语句的变量上（`let e = coded("x"); e.with(…)`），
  // 也可能第一个实参不是字面量。把两者混成一个值，规则要么漏报要么在合法代码上误报。
  it("键名静态判不出来时 keys 为 null（由调用方计数报出，不静默略过）", () => {
    // 没写 with()。
    expect(findCodedCodeLiterals('pub fn f() -> AppError {\n    AppError::coded("a.b")\n}\n')).toEqual([
      { code: "a.b", line: 2, keys: null },
    ]);
    // 分两条语句：with() 挂在变量上，静态判不出它与哪个 coded() 配对——正是本规则刻意不查的形态。
    expect(
      findCodedCodeLiterals('pub fn f() -> AppError {\n    let e = AppError::coded("a.b");\n    e.with("k", v)\n}\n')
    ).toEqual([{ code: "a.b", line: 2, keys: null }]);
    // with() 的实参不是字面量。
    expect(
      findCodedCodeLiterals('pub fn f() -> AppError {\n    AppError::coded("a.b").with(k, v)\n}\n')
    ).toEqual([{ code: "a.b", line: 2, keys: null }]);
    // with() 不是直接链在 coded() 上（中间隔了别的调用）：那几对键不属于这个站点。
    expect(
      findCodedCodeLiterals('pub fn f() -> AppError {\n    wrap(AppError::coded("a.b")).with("k", v)\n}\n')
    ).toEqual([{ code: "a.b", line: 2, keys: null }]);
  });

  // ★ 本任务最容易踩的坑：error.rs 的 `#[cfg(test)]` 块里有三处 `coded("a.b")`。
  //    判据不遮蔽测试项就会把它们当成生产站点，闸门在真实树上报「a.b 未登记」。
  it("`#[cfg(test)]` 测试项里的 coded() 不取出", () => {
    const src =
      'pub fn f() -> AppError {\n    AppError::coded("prod.one")\n}\n\n' +
      "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        let _ = AppError::coded(\"test.only\");\n    }\n}\n";
    expect(findCodedCodeLiterals(src).map((h) => h.code)).toEqual(["prod.one"]);
  });

  // 定义与调用在文本上只差一个 `fn`：`pub fn coded(code: &'static str)` 也长成 `coded(`。
  it("函数定义 `fn coded(` 不是站点，它的第一个参数是形参", () => {
    expect(findCodedCodeLiterals("pub fn coded(code: &'static str) -> Self {\n    todo!()\n}\n")).toEqual([]);
  });

  it("注释与字符串里写着的 coded() 不是站点", () => {
    const src = '// AppError::coded("in.comment")\nconst S: &str = "AppError::coded(\\"in.string\\")";\n';
    expect(findCodedCodeLiterals(src)).toEqual([]);
  });

  // 反面：`(` 与实参之间夹注释是**合法站点**，不能判成「实参不是字面量」。
  // 这一条钉住 skipRustTrivia 真的被走到（只跳空白的实现会在这条上变红）。
  it("`(` 与实参之间夹注释时仍取到字面量", () => {
    const src = 'pub fn f() -> AppError {\n    AppError::coded(/* 见 errors.demo.boom */ "demo.boom")\n}\n';
    expect(findCodedCodeLiterals(src)).toEqual([{ code: "demo.boom", line: 2, keys: null }]);
  });

  it("实参不是字符串字面量时返回 code: null（由调用方报出）", () => {
    const src = "pub fn f(c: &'static str) -> AppError {\n    AppError::coded(c)\n}\n";
    expect(findCodedCodeLiterals(src)).toEqual([{ code: null, line: 2, keys: null }]);
  });

  // ★ 已知**漏报**，方向与上面两条误报相反，也是三者里唯一危险的一个：
  //    改名导入之后调用点写成 `mk("a.b")`，判据只认字面量 `coded(`，一个都扫不到——
  //    那个 code 既不进前向判据、语言包里也不会有条目，用户看到的还是裸 code，
  //    而闸门照常报「通过」。R2 的 `&'static str` 挡不住它（改的是名字不是类型）。
  //    这条**钉住当前的漏报行为**：不是认可它，而是把「它确实漏」变成可测量的——
  //    将来把正则扩到别名，这条会变红，那时改成断言命中即可。本仓今天 0 处。
  it("改名导入的调用点扫不到（已知漏报，钉住当前行为）", () => {
    const src = 'use AppError::coded as mk;\npub fn f() -> AppError {\n    mk("a.b")\n}\n';
    expect(findCodedCodeLiterals(src)).toEqual([]);
  });

  // 真实树上的正面控制：测试装置 `a.b` 与定义 `fn coded` 都不出现，而 T3 登记过的四个
  // code 全部扫得到。断言取**包含**而不是相等——T5 会继续加站点，相等断言会被一次正常
  // 的新增弄红，而红的原因与这条用例要守的东西无关。
  it("本仓真实 Rust 树：a.b 不出现，已登记的四个 code 全部扫得到", () => {
    const root = process.cwd();
    const files = [];
    const walk = (dir) => {
      for (const name of readdirSync(dir)) {
        const full = join(dir, name);
        if (statSync(full).isDirectory()) walk(full);
        else if (full.endsWith(".rs")) files.push(full);
      }
    };
    for (const r of ["src-tauri/src", "src-tauri-proxy/src", "transcript-store/src"]) walk(join(root, r));
    const hits = files.flatMap((f) =>
      findCodedCodeLiterals(readFileSync(f, "utf8")).map((h) => ({ ...h, file: relative(root, f) }))
    );
    expect(hits.filter((h) => h.code === null)).toEqual([]);
    const codes = new Set(hits.map((h) => h.code));
    for (const code of ["streaming.panic", "session.file_missing", "assistant.agent_exit_no_output", "assistant.agent_failed"]) {
      expect(codes.has(code), code).toBe(true);
    }
    expect(codes.has("a.b")).toBe(false);
  });
});

describe("findCodePackMismatches", () => {
  it("前向：站点产出而语言包没有的 code 列进 missing", () => {
    expect(findCodePackMismatches(["a.b", "a.c"], ["a.b"]).missing).toEqual(["a.c"]);
  });

  it("反向：语言包有而没有任何站点产出的键列进 orphaned", () => {
    expect(findCodePackMismatches(["a.b"], ["a.b", "a.d"]).orphaned).toEqual(["a.d"]);
  });

  it("两个方向都闭合时两侧都空", () => {
    expect(findCodePackMismatches(["a.b", "a.b"], ["a.b"])).toEqual({ missing: [], orphaned: [] });
  });

  // 同一 code 被多个站点产出（streaming.panic 实测两处）不改变判据。
  it("重复的 code 不产生重复诊断", () => {
    expect(findCodePackMismatches(["a.b", "a.b"], []).missing).toEqual(["a.b"]);
  });
});

describe("findPlaceholderMismatches", () => {
  const langs = ["en", "zh", "ja", "de"];

  it("某语言漏写占位符时报出", () => {
    const packs = {
      en: { "a.b": "Boom: {path}" },
      zh: { "a.b": "爆炸：{pfad}" },
      ja: { "a.b": "爆発: {path}" },
      de: { "a.b": "Bumm: {path}" },
    };
    expect(findPlaceholderMismatches(packs, langs)).toEqual([
      { key: "a.b", lang: "zh", expected: ["path"], got: ["pfad"] },
    ]);
  });

  it("四语占位符一致时为空", () => {
    const packs = {
      en: { "a.b": "{x} 与 {y}" },
      zh: { "a.b": "{y} 与 {x}" },
      ja: { "a.b": "{x}{y}" },
      de: { "a.b": "{x} und {y}" },
    };
    expect(findPlaceholderMismatches(packs, langs)).toEqual([]);
  });

  // 缺键是规则 3 的管辖：在这里再报一遍会把读者引向两个方向。
  it("某语言缺键时跳过，不报占位符", () => {
    const packs = { en: { "a.b": "{path}" }, zh: {}, ja: { "a.b": "{path}" }, de: { "a.b": "{path}" } };
    expect(findPlaceholderMismatches(packs, langs)).toEqual([]);
  });

  it("无占位符的条目照常比对（多写一个占位符同样报出）", () => {
    const packs = {
      en: { "a.b": "Boom" },
      zh: { "a.b": "爆炸" },
      ja: { "a.b": "爆発 {detail}" },
      de: { "a.b": "Bumm" },
    };
    expect(findPlaceholderMismatches(packs, langs)).toEqual([
      { key: "a.b", lang: "ja", expected: [], got: ["detail"] },
    ]);
  });
});

/**
 * 规则 10 的第四段：**单表达式链**站点的 `with()` 键名与 `en` 条目的 `{…}` 比对。
 *
 * 上面那条 findPlaceholderMismatches 只比四份语言包**彼此**，所以
 * `coded("x").with("pfad", p)` 配上语言包里的 `{path}` 在它那里是**通过**的——而用户看到的
 * 是字面的 `{path}`。这一条补的正是那个缺口：站点提供的键名与语言包声明的占位符必须一一对应。
 */
describe("findCodedWithKeyMismatches", () => {
  const pack = { "a.b": "Boom: {path}", "a.c": "Plain" };

  it("站点键名与语言包占位符不一致时报出（两侧都点名）", () => {
    expect(findCodedWithKeyMismatches([{ code: "a.b", file: "f.rs", line: 2, keys: ["pfad"] }], pack)).toEqual([
      { code: "a.b", file: "f.rs", line: 2, expected: ["path"], got: ["pfad"] },
    ]);
  });

  it("一一对应时为空（顺序无关）", () => {
    expect(
      findCodedWithKeyMismatches(
        [
          { code: "a.b", file: "f.rs", line: 2, keys: ["path"] },
          { code: "a.c", file: "f.rs", line: 9, keys: [] },
        ],
        pack
      )
    ).toEqual([]);
  });

  // 两个方向都要报：语言包声明而站点没提供 → 用户看到字面的 `{path}`；
  // 站点提供而语言包没声明 → 那个参数被丢掉，文案里少了一处上下文。前者更重，后者同样是缺陷。
  it("多一个或少一个占位符都报出", () => {
    expect(
      findCodedWithKeyMismatches([{ code: "a.b", file: "f.rs", line: 2, keys: [] }], pack).map((h) => h.got)
    ).toEqual([[]]);
    expect(
      findCodedWithKeyMismatches([{ code: "a.b", file: "f.rs", line: 2, keys: ["path", "extra"] }], pack).map(
        (h) => h.got
      )
    ).toEqual([["extra", "path"]]);
  });

  // keys 为 null 的站点**跳过**（判不出来），由调用方计数报出——静默略过会让这条规则
  // 看起来比实际覆盖得宽，而那正是「闸门看起来守住了」这一类缺陷。
  it("keys 为 null 的站点跳过", () => {
    expect(findCodedWithKeyMismatches([{ code: "a.b", file: "f.rs", line: 2, keys: null }], pack)).toEqual([]);
  });

  // 缺条目由 findCodePackMismatches 报（前向），不在这里重复报一遍——
  // 两条诊断指向同一个 code 会把读者引向两个方向。
  it("语言包里没有该 code 时跳过", () => {
    expect(findCodedWithKeyMismatches([{ code: "a.zzz", file: "f.rs", line: 2, keys: ["x"] }], pack)).toEqual([]);
  });
});

/**
 * 规则 10 夹具的四份 `errors.json`：键集合四语一致（规则 3 要求），取值按语言给。
 * 与规则 9 的 `fixturePacks` 同形，只是命名空间换成 `errors`——规则 10 只认这一个命名空间。
 */
function fixtureErrorPacks({ en, zh, ja, de }) {
  const file = (o) => `${JSON.stringify(o, null, 2)}\n`;
  return {
    "src/locales/en/errors.json": file(en),
    "src/locales/zh/errors.json": file(zh),
    "src/locales/ja/errors.json": file(ja),
    "src/locales/de/errors.json": file(de),
  };
}

/** 一个 `coded("…")` 字面量站点。夹具里的 Rust 不参与编译，闸门只按文本扫。 */
function codedSite(code, fn = "f") {
  return `pub fn ${fn}() -> AppError {\n    AppError::coded("${code}")\n}\n`;
}

/** 规则 10 的站点文件。三个扫描根都不在夹具白名单里，规则 2 与规则 10 都会读它。 */
const CODED_FILE = "src-tauri/src/demo.rs";

/**
 * 规则 10 的存在理由：`coded()` 的 code 是**前端按 `errors.<code>` 取文案**的键，
 * 而 Rust 侧查不到语言包（T2 的 `coded()` 文档明写）。缺条目时 `renderAppError` 回退成
 * **裸 code 上屏**——用户看到 `system_ops.path_missing`，且没有任何东西会红。
 *
 * **规则 5 不是这条路径的守卫**：`src/utils/invokeApp.ts` 把键提成
 * `const key = \`errors.${err.code}\`` 再传变量，而规则 5 只认字面量/模板实参——
 * 它结构上覆盖不到这里。规则 10 才是。
 *
 * 判据两个方向，缺一不可：
 * - **前向**：站点产出的 code 必须在四份语言包里都有条目（缺了就是裸 code 上屏）；
 * - **反向**：语言包里的每个键必须被某个站点产出。只做前向，语言包就会变成一份
 *   **手工维护的清单**：站点删了、键改名了，条目留在那里没人发现，而清单看起来仍像覆盖。
 *   本仓在「清单看起来像覆盖」上反复付过代价，故反向与正向同等承重。
 */
describe("check-i18n.mjs 规则 10：coded() 的 code 与 errors.* 双向闭合", () => {
  /** 站点产出 demo.boom 与 demo.other；语言包只登记了后者。 */
  const SITES = { [CODED_FILE]: codedSite("demo.boom") + codedSite("demo.other", "g") };
  const PACKS_WITH_OTHER = fixtureErrorPacks({
    en: { demo: { other: "Other" } },
    zh: { demo: { other: "其他" } },
    ja: { demo: { other: "その他" } },
    de: { demo: { other: "Andere" } },
  });
  const PACKS_WITH_BOTH = fixtureErrorPacks({
    en: { demo: { boom: "Boom", other: "Other" } },
    zh: { demo: { boom: "爆炸", other: "其他" } },
    ja: { demo: { boom: "爆発", other: "その他" } },
    de: { demo: { boom: "Bumm", other: "Andere" } },
  });

  it("① 站点产出的 code 不在 errors.json 里 → 退出非 0 并点名该 code", () => {
    const { code, stderr } = runGate({ ...SITES, ...PACKS_WITH_OTHER });
    expect(code).toBe(1);
    expect(stderr).toContain("错误码未登记 errors.demo.boom");
    expect(stderr).toContain(`${CODED_FILE}:2`);
  });

  it("② 补上该条目 → 退出 0", () => {
    const { code, stdout } = runGate({ ...SITES, ...PACKS_WITH_BOTH });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  it("③ errors.json 里有没被任何站点产出的键 → 报出（反向闭合）", () => {
    const { code, stderr } = runGate({
      [CODED_FILE]: codedSite("demo.other"),
      ...fixtureErrorPacks({
        en: { demo: { other: "Other", orphan: "Orphan" } },
        zh: { demo: { other: "其他", orphan: "孤儿" } },
        ja: { demo: { other: "その他", orphan: "孤児" } },
        de: { demo: { other: "Andere", orphan: "Waise" } },
      }),
    });
    expect(code).toBe(1);
    expect(stderr).toContain("errors.demo.orphan");
    expect(stderr).toContain("没有任何 coded() 站点产出");
    expect(stderr).not.toContain("错误码未登记");
  });

  // ④ 四语键集合不一致**已由规则 3 覆盖**（它双向比对命名空间的键集合）。
  //    这一条确认规则 10 不重复报同一件事：两份诊断指向同一个键，会把读者引向两个方向。
  it("④ 四语键集合不一致 → 只由规则 3 报出，规则 10 不重复报", () => {
    const { code, stderr } = runGate({
      [CODED_FILE]: codedSite("demo.boom"),
      ...fixtureErrorPacks({
        en: { demo: { boom: "Boom" } },
        zh: { demo: {} },
        ja: { demo: { boom: "爆発" } },
        de: { demo: { boom: "Bumm" } },
      }),
    });
    expect(code).toBe(1);
    expect(stderr).toContain("语言包缺口 zh/errors.json");
    expect(stderr).not.toContain("错误码未登记");
    expect(stderr).not.toContain("没有任何 coded() 站点产出");
  });

  // ⑤ ★ 这条钉住本任务最容易踩的坑：`src-tauri/src/error.rs` 的 `#[cfg(test)]` 块里有三处
  //    `coded("a.b")` 字面量。判据若不遮蔽测试 span，`a.b` 会被当成生产站点，
  //    闸门在**真实树上**报「a.b 未登记」，而正确的修法是复用 T1 的生产代码切片，
  //    **不是**往四份语言包里塞一个 `a.b`（那是把测试装置污染成语言包条目）。
  it("⑤ `#[cfg(test)]` 测试项里的 coded() 不要求语言包有条目", () => {
    const src =
      'pub fn f() -> AppError {\n    AppError::coded("demo.boom")\n}\n\n' +
      '#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn t() {\n        let _ = AppError::coded("demo.testonly");\n    }\n}\n';
    const { code, stdout } = runGate({
      [CODED_FILE]: src,
      ...fixtureErrorPacks({
        en: { demo: { boom: "Boom" } },
        zh: { demo: { boom: "爆炸" } },
        ja: { demo: { boom: "爆発" } },
        de: { demo: { boom: "Bumm" } },
      }),
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // ⑥ R2 把签名收成 `&'static str`，但那挡不住**变量**：`&'static str` 的形参照样传得进来。
  //    静态判不出它指向哪个 code，就不能假装校验过了——报出来，不静默跳过。
  it("⑥ coded() 的实参是变量 → 报出，不静默跳过", () => {
    const { code, stderr } = runGate({
      [CODED_FILE]: "pub fn f(code: &'static str) -> AppError {\n    AppError::coded(code)\n}\n",
    });
    expect(code).toBe(1);
    expect(stderr).toContain("实参不是字符串字面量");
    expect(stderr).toContain(`${CODED_FILE}:2`);
  });

  // ⑦ 占位符：同一个 key 在四份语言包里的 `{…}` 必须一致。某一份漏写（或改名）时，
  //    该语言下渲染出来的就是字面的 `{path}`——与裸 code 同类，都是把内部标识符推给用户看。
  it("⑦ 同一 key 的四语占位符不一致 → 报出", () => {
    const { code, stderr } = runGate({
      [CODED_FILE]: codedSite("demo.boom"),
      ...fixtureErrorPacks({
        en: { demo: { boom: "Boom: {path}" } },
        zh: { demo: { boom: "爆炸：{pfad}" } },
        ja: { demo: { boom: "爆発: {path}" } },
        de: { demo: { boom: "Bumm: {path}" } },
      }),
    });
    expect(code).toBe(1);
    expect(stderr).toContain("占位符不一致 errors.demo.boom");
  });

  it("⑧ 四语占位符一致 → 退出 0", () => {
    const { code, stdout } = runGate({
      [CODED_FILE]: codedSite("demo.boom"),
      ...fixtureErrorPacks({
        en: { demo: { boom: "Boom: {path}" } },
        zh: { demo: { boom: "爆炸：{path}" } },
        ja: { demo: { boom: "爆発: {path}" } },
        de: { demo: { boom: "Bumm: {path}" } },
      }),
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // ⑨⑩⑪ ★ 占位符缺口（Step 1）：⑧ 那条只证明四份语言包**彼此**一致，
  // 站点 `with("pfad", …)` 配上语言包里的 `{path}` 在它那里照样通过——而用户看到的是字面的
  // `{path}`。判据因此必须把**站点的键名**与语言包声明的占位符比一遍。
  /** 站点与语言包共用的 `{path}` 条目。 */
  const PATH_PACK = fixtureErrorPacks({
    en: { demo: { boom: "Boom: {path}" } },
    zh: { demo: { boom: "爆炸：{path}" } },
    ja: { demo: { boom: "爆発: {path}" } },
    de: { demo: { boom: "Bumm: {path}" } },
  });

  it("⑨ 链式 with(\"pfad\", …) 配语言包里的 {path} → 退出非 0，两侧都点名", () => {
    const { code, stderr } = runGate({
      [CODED_FILE]: 'pub fn f() -> AppError {\n    AppError::coded("demo.boom").with("pfad", "p")\n}\n',
      ...PATH_PACK,
    });
    expect(code).toBe(1);
    expect(stderr).toContain("errors.demo.boom");
    expect(stderr).toContain(`${CODED_FILE}:2`);
    expect(stderr).toContain("pfad");
    expect(stderr).toContain("path");
  });

  it("⑩ 改成 with(\"path\", …) → 退出 0", () => {
    const { code, stdout } = runGate({
      [CODED_FILE]: 'pub fn f() -> AppError {\n    AppError::coded("demo.boom").with("path", "p")\n}\n',
      ...PATH_PACK,
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // 跨行链照旧算同一表达式：本仓 `assistant/commands.rs:441-442`（行号在 `987b91a0` 上量得）的
  // `{status}` 站点就是这种写法，判据若只认同一行，那处真实站点会被当成「没有 with()」，
  // 规则在那里一条抓力都没有。
  it("⑩ 跨行的链照旧校验（本仓真实写法）", () => {
    const { code, stdout } = runGate({
      [CODED_FILE]: 'pub fn f() -> AppError {\n    AppError::coded("demo.boom")\n        .with("path", "p")\n}\n',
      ...PATH_PACK,
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
  });

  // ⑪ ★ 非链式站点（`with()` 挂在另一条语句的变量上）：**不报，但计数**。
  //    判据刻意不查它——静态判不出那个变量是不是这个 code 的错误对象，硬配会造出既漏报又误报的
  //    第二套机制。但「不查」必须**看得见**：闸门输出里的计数就是它，少了那行提示，
  //    批量迁移者会以为 339 条站点全部被这条规则守住了。
  //    夹具刻意让语言包声明 `{path}` 而站点一个键都没提供：把非链式站点当成「零个 with()」的实现
  //    会在这里报出（规则一宽就误报），静默跳过的实现则不会有计数（断言落空）——两种错法各被一半拦住。
  it("⑪ 非链式站点不报，但计数出现在闸门输出里", () => {
    const { code, stdout } = runGate({
      [CODED_FILE]: 'pub fn f() -> AppError {\n    let e = AppError::coded("demo.boom");\n    e.with("path", "p")\n}\n',
      ...PATH_PACK,
    });
    expect(code).toBe(0);
    expect(stdout).toContain("i18n 闸门通过");
    expect(stdout).toContain("1/1 个 coded() 站点的 with() 键名静态判不出来");
    expect(stdout).toContain(`${CODED_FILE}:2`);
  });

  // 链式站点不进计数：计数是「未校验」的度量，把所有站点都数进去等于没有这个度量。
  it("⑪ 的对照：链式站点不进未校验计数", () => {
    const { code, stdout } = runGate({
      [CODED_FILE]: 'pub fn f() -> AppError {\n    AppError::coded("demo.boom").with("path", "p")\n}\n',
      ...PATH_PACK,
    });
    expect(code).toBe(0);
    expect(stdout).not.toContain("静态判不出来");
  });
});
