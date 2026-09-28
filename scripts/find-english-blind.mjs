#!/usr/bin/env node
/**
 * 英文界面文案盲区的【机械派生】测量。
 *
 * 闸门规则 1/2 只认 CJK 码点，**硬编码的英文界面文案对它完全不可见**——
 * 于是「哪些文件含中文」成了各计划任务清单的唯一来源，从来没有中文的文件
 * 被系统性漏掉（见 spec《闸门的结构性盲区：硬编码英文文案》）。
 *
 * 本脚本把那次测量固定下来，取代两个已失效的手工产物：
 *
 * ① 盲文件集合：**白名单从 `check-i18n.mjs` 源码里正则提取**，不手工抄。
 *    手工清单必然漂移——本仓为此付过两次代价（重复键清单 14 vs 实测 91；
 *    英文盲区清单 131/21 在计划 5 把 56 个文件移出白名单之后失效）。
 * ② 候选 dump：对每个盲文件做四类位置的字面量 dump，产出**候选**。
 *
 * **它产出的是候选，不是结论。** 每条命中都要按判据逐条裁定
 * （专有名词、品牌名、格式名、标识符不动；普通名词的 UI 标签要翻），
 * 裁定结果写进 spec 的裁决集——否则下一次测量会把同一批条目重新发现一遍。
 *
 * **裁定结果的机器可读形态是 `scripts/i18n-english-adjudication.json`**（按 `(文件, 文本)` 记，
 * 每条带 `verdict` 与 `reason`）。本脚本每报出一条候选，先在那里查一次：
 * 已在里面的不要再重新裁定；**不在里面的才是新候选**。
 *
 * 用法：
 *   node scripts/find-english-blind.mjs              # dump 候选（按文件分组）
 *   node scripts/find-english-blind.mjs --summary    # 只打印计数
 *   node scripts/find-english-blind.mjs --all        # dump 全部字面量（不筛候选，排错用）
 *   node scripts/find-english-blind.mjs --rekey      # 重键裁决集，并派生规则 8 的允许清单
 *   node scripts/find-english-blind.mjs --rekey --dry-run   # 只打印账目，不写文件
 */
import { readFileSync, readdirSync, renameSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseSfc } from "@vue/compiler-sfc";
import ts from "typescript";
import {
  findCjkLiterals,
  stripJsComments,
  findVueTemplateCjkText,
  findUnlistedEnglishTexts,
  listRule8Files,
  STATIC_TEXT_ATTRS,
} from "./check-i18n.mjs";

/** 裁决集与规则 8 允许清单的仓内路径（相对 `root`）。 */
const ADJUDICATION_REL = "scripts/i18n-english-adjudication.json";
const ALLOWLIST_REL = "scripts/i18n-english-allowlist.json";

/** 白名单数组的正文；`FRONTEND_PENDING` 在闸门源码里是唯一的定义处。 */
const FRONTEND_PENDING_RE = /const FRONTEND_PENDING = \[([\s\S]*?)\];/;

/** 模板正文/文本属性：这些位置的字面量天然面向用户，不另加形状判据。 */
const TEXT_POSITIONS = new Set(["tpl-text"]);

/** 位置是不是「模板正文类」（文本节点与 title/aria-label/placeholder/alt）。 */
function isTextPosition(kind) {
  return TEXT_POSITIONS.has(kind) || kind.startsWith("tpl-attr:");
}

/**
 * 从闸门源码里机械提取前端白名单。
 *
 * **先剥注释再取引号串**：`FRONTEND_PENDING` 的注释里写满了说明，一旦哪天注释里
 * 出现一个带引号的路径，朴素正则会把**注释里的字符串**当成白名单条目——
 * 那个文件随即被从盲文件集合里静默剔除，而输出看起来只是「少了一个文件」。
 * 剥注释复用闸门自己的 `stripJsComments`（同一个事实来源），不另写一份注释语法。
 */
export function extractFrontendPending(gateSource) {
  const m = stripJsComments(gateSource).match(FRONTEND_PENDING_RE);
  if (!m) {
    throw new Error("在闸门源码里找不到 FRONTEND_PENDING——提取规则与闸门脱节了，不能静默返回空表");
  }
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** `src/` 下的前端源文件；与闸门 main() 的过滤条件一致（排除测试文件）。 */
export function listFrontendFiles(root) {
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

/**
 * 「盲文件」判据：不在白名单里，且**两个扫描器都是零**
 * （`findCjkLiterals` 与 `findVueTemplateCjkText`，即闸门规则 1 的全部检测能力）。
 *
 * 两个都要查：只查字面量会把「模板正文里裸写的中文」判成盲文件，
 * 只查模板文本则漏掉脚本区。**双零才是闸门真正看不见的文件。**
 */
export function isBlind(rel, src, pending) {
  if (pending.some((p) => rel.startsWith(p))) return false;
  if (findCjkLiterals(stripJsComments(src)).length > 0) return false;
  if (rel.endsWith(".vue") && findVueTemplateCjkText(src).length > 0) return false;
  return true;
}

/** 派生盲文件集合与基线表的计数。白名单里的文件**两边都不算**（闸门根本没查它）。 */
export function deriveBlindSet(root) {
  const gateSource = readFileSync(join(root, "scripts/check-i18n.mjs"), "utf8");
  const pending = extractFrontendPending(gateSource);
  const files = listFrontendFiles(root);
  const blind = [];
  const nonBlind = [];
  for (const f of files) {
    const rel = relative(root, f).split("\\").join("/");
    if (pending.some((p) => rel.startsWith(p))) continue;
    const src = readFileSync(f, "utf8");
    (isBlind(rel, src, pending) ? blind : nonBlind).push(rel);
  }
  return { total: files.length, pending, blind, nonBlind };
}

/** 绝对偏移 → 行号（1 起）。 */
function lineAt(src, offset) {
  let line = 1;
  for (let i = 0; i < offset && i < src.length; i++) if (src[i] === "\n") line++;
  return line;
}

/**
 * 模板里的文本位置（文本节点、静态文本属性的值）该报哪一行。
 *
 * **报「首个非空白字符所在行」，不是「span 起始行」。** 文本节点常写成
 *
 * ```
 *       <span class="badge">
 *         SYSTEM PROMPT
 *       </span>
 * ```
 *
 * 此时 span 从上一行的换行符起算，`loc.start.line` 比文字实际所在行**早一行**——
 * 报出去的行号在源文件里指不到那串字，读的人按行号核对会对不上。
 * 属性值同理（`title="Git"` 跨行书写时）。
 */
function textPositionLine(src, node) {
  const text = node.content ?? "";
  const lead = text.length - text.trimStart().length;
  return lineAt(src, node.loc.start.offset + lead);
}

/**
 * 一条候选：`kind` 是位置分类，`line` 是源文件里的行号。
 *
 * **`line` 一律指「那串文案的首个非空白字符所在行」**，五类位置口径一致；
 * 文本位置的具体理由见 `textPositionLine`。
 *
 * kind 的取值固定为 `tpl-text` / `tpl-attr:<名>` / `tpl-bind` / `tpl-expr` / `script`：
 *
 * - `tpl-text`：模板文本节点（`type === 2`）
 * - `tpl-attr:<名>`：静态属性（`type === 6` 且名在 `STATIC_TEXT_ATTRS` 里）
 * - `tpl-bind`：**模板字面量**（`` `… ${x} …` ``），无论写在绑定值还是插值里
 * - `tpl-expr`：**引号字面量**，写在模板插值（`type === 5`）或指令表达式（`type === 7` 的 `exp`）里
 * - `script`：`<script>` / `<script setup>` / `.ts` 的正文
 *
 * `tpl-bind` 与 `tpl-expr` 合起来是**补漏层**——这一层在模板里而不在 `<script>` 里，
 * **脚本块扫描覆盖不到**。
 */
function makeItem(kind, line, text) {
  return { kind, line, text };
}

/**
 * 模板字面量的人读形态：准字面量段拼起来，插值处写 `…`。
 *
 * 不吐原始源码（含反引号与 `${}`）——那会把「模板字面量」与「代码片段」在**文本形态**上
 * 混成一类，下游的形状判据只能靠反引号去分辨，而反引号恰好是两者共有的。
 * 插值里的字面量另有遍历单独报出，所以 `…` 不会吞掉信息。
 */
function joinQuasis(quasis, gap = "…") {
  return quasis.map((q) => q.value.cooked ?? q.value.raw ?? "").join(gap);
}

/** TS 的模板字面量：`head` + 各 span 的 `literal`，插值处写 `…`。 */
function joinTsTemplate(node) {
  return [node.head.text, ...node.templateSpans.map((s) => s.literal.text)].join("…");
}

/**
 * 「疑似英文界面文案」的启发式——**产出候选，不是结论**。
 *
 * **位置分两类，判据不同**，理由是两者的信噪比差一个量级：
 *
 * - **模板正文类**（`tpl-text` / `tpl-attr`）：这些位置**只会被渲染给用户看**，
 *   所以只要含字母就算候选（含单字母的 `W`、`v`——它们确实需要一条裁定）。
 * - **表达式与脚本类**（`tpl-bind` / `tpl-expr` / `script`）：绝大多数字面量是代码
 *   （CSS 类名、i18n 键、事件名、协议字段名、import 路径…），必须加形状判据：
 *   含空格、或以大写开头、或以 `:` 结尾。
 *
 * 噪声由**逐条裁定**消化，不在这里无限加过滤器——每加一个过滤器就是加一个事实来源，
 * 而本仓的教训正是「第二个事实来源是缺陷的住处」。
 */
export function isEnglishCandidate(text, kind = "script") {
  const raw = String(text ?? "");
  const t = raw.trim();
  if (!/[A-Za-z]/.test(t)) return false;
  // 代码形态：模板字面量残余、URL、路径、选择器、转义、装饰器
  if (/[`$\\{}@#]|:\/\//.test(t)) return false;
  // 点分小写路径（i18n 键 `app.dialog.error`、域名 `models.dev`）
  if (/^[a-z][a-z0-9-]*(\.[a-z0-9-]+)+$/.test(t)) return false;
  // SVG path data（`M21.751 22.607c1.34 1.005 …`）
  if (/^[Mm][\d\s.,-]{4,}/.test(t)) return false;
  // CSS 颜色与 CSS 函数值（`calc(9px + …)`、`clamp(8px, …)`）
  if (/^rgba?\(/.test(t) || /^#[0-9a-fA-F]{3,8}$/.test(t) || /^[a-z-]+\(/.test(t)) return false;
  // snake_case 标识符（环境变量 `ANTHROPIC_AUTH_TOKEN`、配置键 `experimental_bearer_token`）
  if (/^[A-Za-z][A-Za-z0-9]*(_[A-Za-z0-9]+)+$/.test(t)) return false;
  // 内嵌引号：字体栈、序列化片段
  if (/["']/.test(t)) return false;
  if (isTextPosition(kind)) return true;
  if (!/[A-Za-z]{2}/.test(t)) return false;
  // 空格判据用**未 trim** 的原文：`" [Error]"` 的前导空格是排版的一部分，
  // trim 掉再判会把这条真实候选整条丢掉（原 `reqMessages.ts:245` 就是这么漏的；该字面量
  // 此后已迁入语言包 `common.reqMessages.errorTag`，取值里的前导空格逐字保留）。
  return / /.test(raw) || /^[A-Z]/.test(t) || /:$/.test(t);
}

/**
 * TS AST 遍历。`ctx` 记录「这里是不是日志/模块说明符」——
 * 这两类在闸门与 spec 里都被显式排除（`console.*` 不进语言包、import 路径不是文案），
 * 靠祖先链判断，不能只看直接父节点（`console.warn("a" + "b")` 的字面量父节点是 `+`）。
 */
function walkTs(node, ctx, visit) {
  const flags = {
    log: ctx.log || isConsoleCall(node),
    module: ctx.module || ts.isImportDeclaration(node) || ts.isExportDeclaration(node),
  };
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateExpression(node)) {
    visit(node, { ...flags, parent: ctx.parent });
  }
  ts.forEachChild(node, (c) => walkTs(c, { ...flags, parent: node }, visit));
}

/** `console.<方法>(…)` 的调用表达式。 */
function isConsoleCall(node) {
  if (!ts.isCallExpression(node)) return false;
  const c = node.expression;
  return ts.isPropertyAccessExpression(c) && ts.isIdentifier(c.expression) && c.expression.text === "console";
}

/** 属性名、成员名、import/export 说明符、`require()` 实参——这些位置的字面量是标识符不是文案。 */
function isIdentifierPosition(node, parent) {
  if (!parent) return false;
  if (ts.isPropertyAssignment(parent) && parent.name === node) return true;
  if (ts.isPropertyAccessExpression(parent) && parent.name === node) return true;
  if (ts.isElementAccessExpression(parent) && parent.argumentExpression === node) return true;
  if (ts.isImportDeclaration(parent) || ts.isExportDeclaration(parent)) return true;
  if (ts.isImportSpecifier(parent) || ts.isExportSpecifier(parent)) return true;
  if (ts.isExternalModuleReference(parent)) return true;
  if (ts.isCallExpression(parent) && ts.isIdentifier(parent.expression) && parent.expression.text === "require") {
    return true;
  }
  return false;
}

/**
 * 一个文件里的全部字面量，按位置分类。
 *
 * `.vue` 走 SFC：模板用 `@vue/compiler-sfc` 的 AST，脚本区用 TS AST。
 * `.ts` 整份当脚本区扫。
 */
export function scanLiterals(rel, src, diag = { unparsed: [] }) {
  const out = [];
  if (!rel.endsWith(".vue")) {
    const sf = ts.createSourceFile(rel, src, ts.ScriptTarget.Latest, true);
    walkTs(sf, { log: false, module: false, parent: null }, (node, ctx) => {
      if (ctx.log || ctx.module || isIdentifierPosition(node, ctx.parent)) return;
      const line = lineAt(src, node.getStart(sf));
      out.push(makeItem("script", line, ts.isTemplateExpression(node) ? joinTsTemplate(node) : node.text));
    });
    return out;
  }

  const { descriptor } = parseSfc(src, { filename: rel });
  const walkTemplate = (node) => {
    if (!node || typeof node !== "object") return;
    switch (node.type) {
      case 0: // ROOT
        (node.children ?? []).forEach(walkTemplate);
        break;
      case 1: // ELEMENT
        for (const p of node.props ?? []) {
          if (p.type === 6) {
            if (STATIC_TEXT_ATTRS.has(p.name) && p.value) {
              out.push(makeItem(`tpl-attr:${p.name}`, textPositionLine(src, p.value), p.value.content));
            }
          } else if (p.type === 7) {
            scanTemplateExpression(p.exp, src, out, diag);
          }
        }
        (node.children ?? []).forEach(walkTemplate);
        break;
      case 2: // TEXT
        out.push(makeItem("tpl-text", textPositionLine(src, node), node.content));
        break;
      case 3: // COMMENT
        break;
      case 5: // INTERPOLATION
        scanTemplateExpression(node.content, src, out, diag);
        break;
      default:
        diag.unhandled.add(node.type);
    }
  };
  if (descriptor.template?.ast) walkTemplate(descriptor.template.ast);

  for (const block of [descriptor.scriptSetup, descriptor.script]) {
    if (!block) continue;
    const sf = ts.createSourceFile(rel + ".ts", block.content, ts.ScriptTarget.Latest, true);
    const offset = block.loc.start.offset;
    walkTs(sf, { log: false, module: false, parent: null }, (node, ctx) => {
      if (ctx.log || ctx.module || isIdentifierPosition(node, ctx.parent)) return;
      const line = lineAt(src, offset + node.getStart(sf));
      out.push(makeItem("script", line, ts.isTemplateExpression(node) ? joinTsTemplate(node) : node.text));
    });
  }
  return out;
}

/**
 * 模板表达式里的引号字面量。
 *
 * 优先用 `exp.ast`（编译器已解析好的 babel AST，含相对偏移）；没有时退回
 * 「拿 `exp.content` 自己解析一遍」。两者都失败就**记一笔 unparsed**——
 * 静默跳过会让「这一层扫过了」变成假话。
 */
function scanTemplateExpression(exp, src, out, diag) {
  if (!exp) return;
  const base = exp.loc?.start?.offset;
  if (exp.ast && typeof base === "number") {
    walkBabel(exp.ast, (node) => {
      if (node.type === "StringLiteral") {
        out.push(makeItem("tpl-expr", lineAt(src, base + node.start), node.value));
      } else if (node.type === "TemplateLiteral") {
        out.push(makeItem("tpl-bind", lineAt(src, base + node.start), joinQuasis(node.quasis)));
      }
    });
    return;
  }
  if (typeof exp.content === "string" && typeof base === "number") {
    try {
      const sf = ts.createSourceFile("expr.ts", exp.content, ts.ScriptTarget.Latest, true);
      walkTs(sf, { log: false, module: false, parent: null }, (node, ctx) => {
        if (ctx.log || ctx.module) return;
        if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
          out.push(makeItem("tpl-expr", lineAt(src, base + node.getStart(sf)), node.text));
        } else if (ts.isTemplateExpression(node)) {
          out.push(makeItem("tpl-bind", lineAt(src, base + node.getStart(sf)), joinTsTemplate(node)));
        }
      });
    } catch {
      diag.unparsed.push(exp.content);
    }
    return;
  }
  diag.unparsed.push(String(exp.content ?? ""));
}

/** babel AST 遍历（只看带 `type` 的对象/数组字段，`loc`/`start`/`end` 不是子节点）。 */
function walkBabel(node, visit) {
  if (!node || typeof node !== "object") return;
  if (typeof node.type === "string") visit(node);
  for (const key of Object.keys(node)) {
    if (key === "loc" || key === "start" || key === "end" || key === "extra") continue;
    const v = node[key];
    if (Array.isArray(v)) v.forEach((c) => walkBabel(c, visit));
    else if (v && typeof v === "object" && typeof v.type === "string") walkBabel(v, visit);
  }
}

/** 跑完整测量：盲文件集合 + 每个盲文件的候选。 */
export function measure(root, { all = false } = {}) {
  const set = deriveBlindSet(root);
  const diag = { unparsed: [], unhandled: new Set() };
  const withCandidates = [];
  let candidateCount = 0;
  for (const rel of set.blind) {
    const src = readFileSync(join(root, rel), "utf8");
    const items = scanLiterals(rel, src, diag);
    const hits = all ? items : items.filter((i) => isEnglishCandidate(i.text, i.kind));
    if (hits.length) {
      withCandidates.push({ rel, items: hits });
      candidateCount += hits.length;
    }
  }
  return { ...set, withCandidates, candidateCount, diag };
}

/** 读仓内裁决集。 */
export function readAdjudication(root) {
  return JSON.parse(readFileSync(join(root, ADJUDICATION_REL), "utf8"));
}

/**
 * 候选 `(rel, text)` 是否已被裁决。
 *
 * **判据是「这段文本在这个文件里」，不是「这个行号上有什么」**：文本是身份，行号只是它
 * 当前所在的位置。按行号记的键会在任何一次插入/删除之后烂掉——实测改造前那一版
 * （`47ffc0d4`，274 条；`git show 47ffc0d4:scripts/i18n-english-adjudication.json`），**两个口径**：
 * 按「所引行号上还有没有候选」量得 **40 条已不对应任何候选**（死键；今天 `_orphans` 下正好是
 * 这 40 个键）；按「所引行号的原文是否仍逐字包含该文本」量得 **72 条已不包含**——其中 **64 条**
 * 该文本已整个不在文件里、**8 条**文本还在文件里但搬到了别的行。**两个口径不可相减**：
 * 那 40 条死键里有 3 条所引行号的原文仍含该文本，故死键不是陈旧的子集，引用时必须连口径一起引。
 *
 * `#N` 兜底：同一文件里出现**完全相同的文本**、且两条判断不同（理由不同）时，第 2 处起
 * 带 `#N` 后缀（见 `rekey`）。后缀只是出现序的区分，故查找时按去掉后缀的基名比对。
 */
export function isAdjudicated(adj, rel, text) {
  const byText = adj[rel];
  if (!byText) return false;
  if (Object.hasOwn(byText, text)) return true;
  return Object.keys(byText).some((k) => k.replace(/#\d+$/, "") === text);
}

/**
 * 本次测量里**不在裁决集里**的候选，每项形如 `` `${rel}:${line} ${JSON.stringify(text)}` ``。
 *
 * 判据（`isAdjudicated`）与 `--rekey`、`find-english-blind.test.mjs` 的仓库现状断言**同源**：
 * 三处各写一遍会让「什么算已裁决」出现三个事实来源，而裁决集的形态正是本任务要改的东西。
 */
export function unadjudicated(root) {
  const adj = readAdjudication(root);
  const r = measure(root);
  const out = [];
  for (const f of r.withCandidates) {
    for (const i of f.items) {
      if (!isAdjudicated(adj, f.rel, i.text)) out.push(`${f.rel}:${i.line} ${JSON.stringify(i.text)}`);
    }
  }
  return out;
}

/**
 * 裁决集的两种形态归一成同一张条目表（每项 `{ rel, text, verdict, reason, origin }`）：
 *
 * - **新形态**（本任务之后）：顶层键是文件路径，值是 `文本 → { verdict, reason }`；
 * - **旧形态**（本任务之前）：顶层键是 `path:line[#N]`，值是 `{ text, verdict, reason }`。
 *
 * 旧形态仍认，是为了让 `--rekey` 在改造前的树上能跑；两种都读，还让改造后的树上**跑第二遍
 * 是幂等的**（同一个键、同一个理由，重新定位到同一处）。`_orphans` 里的条目照旧读入——
 * 它们仍是人的判断，只是那串文本当前不在该文件的候选里。
 */
export function normalizeAdjudication(raw) {
  const out = [];
  for (const [k, v] of Object.entries(raw)) {
    if (k === "_orphans") continue;
    if (!v || typeof v !== "object") continue;
    if ("verdict" in v) {
      out.push({
        rel: k.slice(0, k.lastIndexOf(":")),
        text: v.text,
        verdict: v.verdict,
        reason: v.reason,
        origin: k,
      });
    } else {
      for (const [t, e] of Object.entries(v)) {
        out.push({ rel: k, text: t.replace(/#\d+$/, ""), verdict: e.verdict, reason: e.reason, origin: `${k}:${t}` });
      }
    }
  }
  for (const [k, v] of Object.entries(raw._orphans ?? {})) {
    out.push({ rel: k.slice(0, k.lastIndexOf(":")), text: v.text, verdict: v.verdict, reason: v.reason, origin: k });
  }
  return out;
}

/**
 * 规则 8 允许清单（`{ 文本: 理由 }`）的机械派生。
 *
 * **判据是位置，不是文本本身**：只有「规则 8 真会命中」的 keep 才进清单。判据与闸门同源——
 * 直接调闸门自己的 `findUnlistedEnglishTexts`（空 allowlist ⇒ 返回它扫到的每个文本），
 * 不另写一套形状过滤：那是第二套事实来源，两边一旦分叉，清单里的条目会与闸门实际取到的
 * 文本对不上，逐条落空而输出看起来只是「清单里某条没用了」。
 *
 * 扫描域取**整个 `src/` 的 `.vue`**，不是「keep 自己所在的那个文件」：
 * 清单要保证的是「规则 8 报得出的文本都在清单里」，而规则 8 是按文件报的——按 keep 所在文件
 * 逐个判，一个 keep 若只落在 `.ts` 里、而同一文本出现在别的 `.vue` 模板里，那条文本就会漏发，
 * 闸门随即变红。**实测两种口径在当前树上给出同一个集合**（38 个文本），取不会漏的那种。
 * 文件集合与过滤条件**直接取闸门导出的 `listRule8Files`**（`check-i18n.mjs` 里规则 8 那一轮
 * walk 用的就是它，今天 111 个文件），不在这里另写一遍 walk——那会是第二份事实来源，闸门哪天
 * 改了过滤条件，清单会静默地与它错开。
 *
 * 脚本区（`<script>` 与 `.ts`）的 keep 规则 8 一条都够不到，列进清单只会让清单看起来比
 * 实际抓力大——与规则 9 的允许清单同一条取舍（见 `gen-drift-allowlist.mjs`），故由本函数
 * 过滤掉并交出 `skipped` 供逐条核对。**这些 keep 的判断本身没有丢**：它们仍在裁决集里，
 * 由「重跑本脚本」这条通道兜（规则 8 的具名限制，见闸门里 `findUnlistedEnglishTexts` 的说明）。
 *
 * 同一文本有多条 keep 判断时**取先出现的那条**；理由不止一种的文本由 `reasonedTwice` 交出，
 * 由 `--rekey` 打印——清单的每个值只能有一个理由，这个选择要让人看得见。
 */
export function deriveEnglishAllowlist(root, adjudication) {
  const firing = new Set();
  for (const f of listRule8Files(root)) {
    for (const t of findUnlistedEnglishTexts(readFileSync(f, "utf8"), {})) firing.add(t);
  }
  const allowlist = {};
  const emitted = [];
  const skipped = [];
  const reasonedTwice = [];
  const reasonsByText = new Map();
  const seen = new Set();
  for (const e of normalizeAdjudication(adjudication)) {
    if (e.verdict !== "keep") continue;
    const text = e.text.trim();
    if (!reasonsByText.has(text)) reasonsByText.set(text, new Set());
    reasonsByText.get(text).add(e.reason);
    if (seen.has(text)) continue;
    seen.add(text);
    if (!firing.has(text)) {
      skipped.push(text);
      continue;
    }
    allowlist[text] = e.reason;
    emitted.push(text);
  }
  for (const [text, reasons] of reasonsByText) {
    if (reasons.size > 1 && Object.hasOwn(allowlist, text)) reasonedTwice.push({ text, reasons: [...reasons] });
  }
  const sorted = {};
  // 码位序，不是 `localeCompare`：这是**提交进仓的派生文件**，任何一台机器上重跑都必须得到
  // 逐字节相同的结果，而 `localeCompare` 随运行环境的 locale 变——实测本机（LANG=zh_CN）
  // 与码位序不同（`⌘D` 会排到最前）。
  for (const t of Object.keys(allowlist).sort()) sorted[t] = allowlist[t];
  return { allowlist: sorted, emitted, skipped, reasonedTwice };
}

/**
 * 按 `(文件, 文本)` 重键裁决集，并重新派生规则 8 的允许清单。三件事：
 *
 * 1. **搬键**：对每条现有条目，在对应文件里按 `text` 找它的当前位置，写进新结构。
 * 2. **合并重复**：同一 `(文件, 文本)` 出现多条时合成一条。**`verdict` 冲突则停下报告**
 *    （不替人做选择，也不写文件）；**理由不同的不合并**——合并会把一条人写过的判断丢掉，
 *    那种情形按出现序留成 `文本`、`文本#2`…，与「同一文件里同一文本有多处」是同一件事。
 * 3. **报出孤儿**：`text` **不在该文件的候选里**的条目不静默删除——打印出来，并**原样**保留在
 *    `_orphans` 下。判据是「不在候选里」，**不是**「文本在文件里找不到」：文本还在、只是不再是
 *    候选（没过 `isEnglishCandidate` 的形状判据），或整个文件已不再是盲文件，同样会落进来。
 *    故 `_orphans` **不等于「死文本」**——实测今天 40 条里 2 条（`models.dev`、
 *    `Co-Authored-By: …`）仍留在文件里，且仍作为 keep 进规则 8 的允许清单。
 *
 * 冲突存在时**一个文件都不写**：半写会让裁决集停在两种形态之间。
 */
export function rekey(root, { dryRun = false } = {}) {
  const entries = normalizeAdjudication(readAdjudication(root));
  const r = measure(root);
  const itemsByFile = new Map(r.withCandidates.map((f) => [f.rel, f.items]));

  const groups = new Map();
  for (const e of entries) {
    const key = JSON.stringify([e.rel, e.text]);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(e);
  }

  const adjudication = {};
  const orphans = {};
  const conflicts = [];
  const merged = [];
  const keptApart = [];
  let moved = 0;
  // 不变量：同一个文件里，一个键只会被写一次。两条来路都封死——
  // ① 直写 `text`：分组键就是 `(文件, 文本)`，同组只写一次，不同组的文本必不相同。
  // ② 拆写 `text#N`：只有「理由不同」那条规则会生成后缀，而两个不同的组不可能生成同一个
  //    `T#N`（那要求两组的文本相同）；真实文本也**不可能**等于 `T#N`——候选文本里含 `#` 的
  //    会被 `isEnglishCandidate` 的 `[`$\\{}@#]` 那一条直接滤掉（逐位置类别实测：tpl-text /
  //    tpl-attr / tpl-bind / tpl-expr / script 六类一律 false），故它进不了 ① 这一支。
  // 旧形态没有这个不变量：那时文本是**值**，不是键，撞不了。**若哪天放宽那条形状判据**，
  // 这里会开始互相覆盖并静默丢掉一条人的判断——故把它写在这里，而不是加一个永不触发的守卫。
  const put = (rel, key, e) => {
    (adjudication[rel] ??= {})[key] = { verdict: e.verdict, reason: e.reason };
  };

  for (const es of groups.values()) {
    const { rel, text } = es[0];
    if (!(itemsByFile.get(rel) ?? []).some((i) => i.text === text)) {
      for (const e of es) orphans[e.origin] = { text: e.text, verdict: e.verdict, reason: e.reason };
      continue;
    }
    if (new Set(es.map((e) => e.verdict)).size > 1) {
      conflicts.push({ rel, text, entries: es });
      continue;
    }
    moved += es.length;
    if (new Set(es.map((e) => e.reason)).size === 1) {
      if (es.length > 1) merged.push({ rel, text, n: es.length });
      put(rel, text, es[0]);
    } else {
      keptApart.push({ rel, text, n: es.length });
      es.forEach((e, i) => put(rel, i === 0 ? text : `${text}#${i + 1}`, e));
    }
  }

  const fileKeys = Object.values(adjudication).reduce((n, m) => n + Object.keys(m).length, 0);
  if (Object.keys(orphans).length) adjudication._orphans = orphans;
  const allow = deriveEnglishAllowlist(root, adjudication);
  const written = !dryRun && conflicts.length === 0;
  if (written) {
    // 这两个文件是「裁决集 → 投影」的一对，直接各写一次的话，两次 writeFileSync 之间失败就会
    // 留下一个已重键、另一个还是旧内容的树（闸门读的是投影，它照常绿，而两者已经对不上）。
    // 先把两份内容都写进临时文件，再依次 rename：rename 是原子的，故每个目标文件要么是旧内容、
    // 要么是完整的新内容，不会出现半截；失败点只剩两次 rename 之间。
    const payloads = [
      [ADJUDICATION_REL, `${JSON.stringify(adjudication, null, 2)}\n`],
      [ALLOWLIST_REL, `${JSON.stringify(allow.allowlist, null, 2)}\n`],
    ];
    for (const [rel, body] of payloads) writeFileSync(join(root, `${rel}.tmp`), body);
    for (const [rel] of payloads) renameSync(join(root, `${rel}.tmp`), join(root, rel));
  }
  return { adjudication, orphans, conflicts, merged, keptApart, moved, fileKeys, allow, written };
}

/** `--rekey` 的账目。四行数（搬键/合并/孤儿/allowlist 发出）是要一眼看到的，其余供核对。 */
function reportRekey(root, argv) {
  const dryRun = argv.includes("--dry-run");
  const res = rekey(root, { dryRun });
  // 冲突先判、先报：此时一个文件都没写，下面的账目描述的是一个**没有落盘**的中间态，
  // 打出来会让人以为已经写进去了。
  if (res.conflicts.length) {
    console.error(`⛔ ${res.conflicts.length} 组 (文件, 文本) 的 verdict 冲突，未写任何文件：`);
    for (const c of res.conflicts) {
      console.error(`  ${c.rel} ${JSON.stringify(c.text)}`);
      for (const e of c.entries) console.error(`    ${e.origin} ${e.verdict}：${JSON.stringify(e.reason)}`);
    }
    return 1;
  }
  console.log(`搬键 ${res.moved} 条 → ${res.fileKeys} 个键`);
  console.log(`合并 ${res.merged.length} 组`);
  console.log(`孤儿 ${Object.keys(res.orphans).length} 条`);
  console.log(`allowlist 发出 ${res.allow.emitted.length} 条`);
  // 这个数的总体是 `deriveEnglishAllowlist` 遍历的那一份：`normalizeAdjudication` 读入的**全部**
  // keep 文本——**活条目 + `_orphans`**——按 `text.trim()` 去重。**只数活条目会得到更小的数**
  // （未去空白 148、去空白 145），两者都不是这一行要报的量；口径不写出来，下一个人拿 148 来核
  // 这一行就会以为账目错了。
  console.log(`（keep 文本 ${res.allow.emitted.length + res.allow.skipped.length} 个` +
    `——活条目 + 孤儿里的 keep、去空白后去重；规则 8 够不到、不发 ${res.allow.skipped.length} 个）`);
  if (res.keptApart.length) {
    console.log(`同一文本、理由不同、按出现序保留后缀 ${res.keptApart.length} 组：`);
    for (const g of res.keptApart) console.log(`  ${g.rel} ${JSON.stringify(g.text)} ×${g.n}`);
  }
  if (res.merged.length) {
    console.log("合并的组（同一文件里同一文本、判断逐字相同）：");
    for (const g of res.merged) console.log(`  ${g.rel} ${JSON.stringify(g.text)} ×${g.n}`);
  }
  console.log("孤儿（文本不在该文件的候选里，原样保留在 _orphans 下）：");
  for (const [k, v] of Object.entries(res.orphans)) console.log(`  ${k} ${JSON.stringify(v.text)} [${v.verdict}]`);
  if (res.allow.skipped.length) {
    console.log("不发（规则 8 够不到：脚本区，或当前不在模板位置）的 keep 文本，逐条列出供核对：");
    for (const t of res.allow.skipped) console.log(`  ${JSON.stringify(t)}`);
  }
  for (const { text, reasons } of res.allow.reasonedTwice) {
    console.log(`⚠️ 清单值有多条理由可选，取先出现的那条：${JSON.stringify(text)}`);
    for (const r of reasons) console.log(`    ${JSON.stringify(r)}`);
  }
  console.log(dryRun ? "（--dry-run：未写文件）" : `已写 ${ADJUDICATION_REL} 与 ${ALLOWLIST_REL}`);
  return 0;
}

function main() {
  const root = fileURLToPath(new URL("..", import.meta.url));
  const argv = process.argv.slice(2);
  if (argv.includes("--rekey")) {
    process.exitCode = reportRekey(root, argv);
    return;
  }
  const all = argv.includes("--all");
  const r = measure(root, { all });

  if (argv.includes("--summary")) {
    console.log(`total=${r.total}`);
    console.log(`pending=${r.pending.length}`);
    console.log(`blind=${r.blind.length}`);
    console.log(`nonBlind=${r.nonBlind.length}`);
    console.log(`candidateFiles=${r.withCandidates.length}`);
    console.log(`candidates=${r.candidateCount}`);
    return;
  }

  for (const f of r.withCandidates) {
    console.log(`\n${f.rel}`);
    for (const i of f.items) console.log(`  ${i.line}\t${i.kind}\t${JSON.stringify(i.text)}`);
  }
  console.log(
    `\n# total=${r.total} pending=${r.pending.length} blind=${r.blind.length} ` +
      `candidateFiles=${r.withCandidates.length} candidates=${r.candidateCount}`
  );
  if (r.diag.unparsed.length) console.error(`# 无法解析的模板表达式 ${r.diag.unparsed.length} 处`);
  if (r.diag.unhandled.size) console.error(`# 未处理的模板节点类型 ${[...r.diag.unhandled].join(",")}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
