#!/usr/bin/env node
/**
 * `#[cfg(...windows...)]` 门控站点的【形状审计】。
 *
 * ⚠️ **它证明不了「Windows 上编译不过」，也不是验证。**
 * 本机跑不了 Windows 目标的 cargo check：`ring` / `zstd-sys` / `libz-sys` 的 C 构建
 * 需要 Windows SDK 头文件，实测卡在 `fatal error: 'assert.h' file not found`。
 * 所以这里拿不到任何类型信息，本脚本只做一件事：把「可能把一个 String/&str 错误
 * 转成 AppError」的**形状**捞出来，交给人逐条读。
 *
 * **候选清单不是结论。把候选清单说成验证，就是本仓库反复修的那类缺陷。**
 *
 * 判据（脚本自己打印，便于复现，不要凭记忆引用）：
 *   扫描根：`git ls-files src-tauri/src` 下的 `.rs`。**`src-tauri-proxy` 不在射程内**，
 *        它也有 `#[cfg(windows)]`（`src-tauri-proxy/src/lifecycle.rs:10,40`），本脚本不扫。
 *   射程：cfg 谓词在 `target_os = "windows"` 下求值为真的区域（含 `any(...)` / `not(...)`）。
 *        **每一条非注释行的 `#[cfg(` 都在 `TARGET` 下求值，没有预过滤** —— 谓词里有没有
 *        `windows` 这个词，与它在 Windows 上是否为真无关：`not(target_os = "macos")`
 *        与 `not(unix)`（后者是「Windows 专有」的惯用写法）都不含这个词，却在 Windows 上为真。
 *   形状：射程内、非注释行（剔除 `{...}` 格式串后）出现 `?`、`.into()`、
 *        `AppError::from`、`From::from` 之一。
 *   账目（脚本自己断言并打印）：`Windows 真 + Windows 假 + 无法求值 = cfg 属性总数`。
 *        **三桶必须凑成总数**，否则「没扫到」就会被读成「扫过且干净」——
 *        本脚本第一版按「谓词里有没有 `windows`」预过滤，把 210 条 cfg 里的 133 条
 *        丢在账外，还把剩下的一部分报成「Windows 上不编译的全部」（那个数不是总数，
 *        也不是射程外区域数）。与 `probe-detail-args.mjs` 的
 *        `codeOpeners === injected + unreached` 是同一个形状。
 *
 * 形状匹配的已知盲区（**不要**把它读成已覆盖）：裸 `impl Into<AppError>` 的实参位
 * （即形参声明成 `impl Into<AppError>` 而不写 `.into()`）不产生 `.into()` 字面量，
 * 本脚本匹配不到，只能靠人读。
 *
 * 只做形状匹配、不做返回类型推断：`-> AppResult` 的启发式会漏掉签名跨行、
 * `impl` 块内方法等情形，漏掉的那些恰恰可能是 Windows 独有的 `?`。
 */
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const ROOT = process.cwd();
const files = execFileSync("git", ["ls-files", "src-tauri/src"], { cwd: ROOT, encoding: "utf8" })
  .trim().split("\n").filter((f) => f.endsWith(".rs"));

// 只问「这段代码在 Windows 上会不会被编译」：test / unix 均取假。
const TARGET = { target_os: "windows", windows: true, unix: false, test: false };

/** 按顶层逗号切分，`any(a, not(b))` → ["any(a, not(b))"] 内的参数。 */
function splitTop(s) {
  const out = []; let depth = 0, cur = "";
  for (const ch of s) {
    if (ch === "(") depth++;
    if (ch === ")") depth--;
    if (ch === "," && depth === 0) { out.push(cur.trim()); cur = ""; continue; }
    cur += ch;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

/**
 * 求值 cfg 谓词，三值逻辑：true / false / **null = 认不出，未扫**。
 *
 * ⚠️ 认不出的键**必须返回 null，不能返回 false**。返回 false 会把
 * `all(windows, target_env = "msvc")` 这类谓词**静默**归入射程外且不加任何计数，
 * 于是「未扫: 0」变成由构造保证的 0，读者会把「没扫到」读成「扫过且干净」。
 * null 必须能穿过 `not` / `any` / `all` 传播到调用点。
 */
function evalPred(p, env) {
  p = p.trim();
  if (p.startsWith("not(")) {
    const v = evalPred(p.slice(4, -1), env);
    return v === null ? null : !v;
  }
  if (p.startsWith("any(")) {
    const vs = splitTop(p.slice(4, -1)).map((x) => evalPred(x, env));
    if (vs.includes(true)) return true;
    return vs.includes(null) ? null : false;
  }
  if (p.startsWith("all(")) {
    const vs = splitTop(p.slice(4, -1)).map((x) => evalPred(x, env));
    if (vs.includes(false)) return false;
    return vs.includes(null) ? null : true;
  }
  const m = p.match(/^(\w+)\s*=\s*"([^"]*)"$/);
  if (m) return m[1] in env ? env[m[1]] === m[2] : null; // 认不出的键 → null
  if (p in env) return env[p];
  return null; // 认不出的谓词 → null
}

/** 取出 `#[cfg(` 之后配平的那段谓词文本。 */
function predOf(line, openParen) {
  let depth = 0, start = null;
  for (let i = openParen; i < line.length; i++) {
    const ch = line[i];
    if (ch === "(") { depth++; if (depth === 1) start = i + 1; }
    else if (ch === ")") { depth--; if (depth === 0) return line.slice(start, i); }
  }
  return null;
}

/** item 正文：有花括号则配平到深度 0；无花括号（如 `use ...;`）则到 `;` 为止。 */
function itemBody(lines, start) {
  let depth = 0, seen = false, out = [];
  for (let i = start; i < lines.length; i++) {
    const l = lines[i];
    for (const ch of l) { if (ch === "{") { depth++; seen = true; } else if (ch === "}") depth--; }
    out.push({ line: i + 1, text: l });
    if (!seen && /;\s*$/.test(l)) break;
    if (seen && depth === 0) break;
  }
  return out;
}

const SHAPES = [
  [/\?/, "`?`"],
  [/\.into\(\)/, ".into()"],
  [/AppError::from/, "AppError::from"],
  [/From::from/, "From::from"],
];

let total = 0, windowsTrue = 0, windowsFalse = 0, unknown = 0;
const candidates = [];
const unknownPreds = [];
for (const f of files) {
  const lines = readFileSync(f, "utf8").split("\n");
  for (let i = 0; i < lines.length; i++) {
    const at = lines[i].indexOf("#[cfg(");
    if (at < 0) continue;
    if (/^\s*(\/\/|\/\*)/.test(lines[i])) continue; // 注释里提到属性，不是门控站点
    // 每一条属性都先入账再分流：下面每个 `continue` 都必须落在某一个桶里，
    // 任何一条都不许被静默丢掉（丢一条，总数就不再是总数）。
    total++;
    const pred = predOf(lines[i], at + 5);
    if (pred === null) {
      // `#[cfg(` 不在本行闭合，取不到谓词文本，**无法判断它是否与 windows 有关**，
      // 故计入「未扫」并打印，而不是当作「与 windows 无关」静默跳过。
      unknown++; unknownPreds.push(`${f}:${i + 1}  (谓词不闭合于本行)`);
      continue;
    }
    // 不求值就按「谓词里有没有 windows」跳过，会把 `not(unix)` 这类 Windows 专有写法
    // 整片丢出射程且不加任何计数 —— 那正是本脚本要关掉的缺陷。故一律求值。
    const v = evalPred(pred, TARGET);
    if (v === null) { unknown++; unknownPreds.push(`${f}:${i + 1}  ${pred}`); continue; }
    if (!v) { windowsFalse++; continue; }
    windowsTrue++;
    for (const b of itemBody(lines, i)) {
      if (/^\s*(\/\/|\/\*)/.test(b.text)) continue;
      // 剔除 `{...}` 格式串，避免把 `{:?}` 当成 `?` 运算符
      const code = b.text.replace(/\{[^{}]*\}/g, "");
      for (const [re, name] of SHAPES) {
        if (re.test(code)) {
          candidates.push({ f, line: b.line, why: name, text: b.text.trim() });
          break;
        }
      }
    }
  }
}

// 账目恒等式：三条分流互斥且穷尽，故必须等于属性总数。它由构造保证，
// 但**必须在这里断言** —— 对不上账时，「还有几条没扫」就没有下界，
// 而那正是「静默跳过然后报干净」的入口。
const balanced = windowsTrue + windowsFalse + unknown === total;
if (!balanced) {
  console.error(
    `⛔ 射程账目不平：Windows 真 ${windowsTrue} + Windows 假 ${windowsFalse} + 未扫 ${unknown} ≠ cfg 属性总数 ${total}。`
  );
  process.exit(1);
}

console.log(`扫描根: git ls-files src-tauri/src（${files.length} 个 .rs）—— src-tauri-proxy 不在射程内`);
console.log('射程判据：cfg 谓词在 target_os="windows" 下求值为真（test/unix 取假）');
console.log(`cfg 属性总数（非注释行的 \`#[cfg(\`）: ${total}`);
console.log(`  在 Windows 上为真 ⇒ 射程内，逐条扫形状: ${windowsTrue}`);
console.log(`  在 Windows 上为假 ⇒ 不参与编译，未扫形状: ${windowsFalse}`);
console.log(`  无法求值 / 谓词不闭合于本行 ⇒ 未扫形状: ${unknown}`);
console.log(`账目: ${windowsTrue} + ${windowsFalse} + ${unknown} = ${total} ✅`);
for (const u of unknownPreds) console.log(`  ? ${u}`);
console.log(`候选数: ${candidates.length}  —— 候选不是结论，逐条读`);
for (const c of candidates) console.log(`  ${c.f}:${c.line}  [${c.why}]  ${c.text}`);
