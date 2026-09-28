#!/usr/bin/env node
/**
 * 编译器探针：`.with("detail", X.to_string())` 站点里的 `X` 是不是 `AppError`？
 *
 * **为什么要问这个问题**：`AppError::Coded` 的 `Display` 是**裸 code**（`#[error("{code}")]`），
 * 所以 `X` 若是 `AppError`，`.to_string()` 取到的就是 code 本身 —— `detail` 里装进
 * 「db.tantivy_index_open_failed」这样的码，内层真正的第三方原文被丢弃，用户屏幕上出现
 * 「…: db.tantivy_index_open_failed」。反过来，`X` 若是第三方错误（`io::Error` /
 * `git2::Error` / `serde_json::Error`…），`to_string()` 就是正确用法。
 *
 * ## 判据来自编译器，不是命名启发式
 *
 * 打补丁的形态（**全部在 /tmp 之外的原地做，跑完 `--revert` 还原**）：
 *
 * 1. 射程内每个站点，`X.to_string()` → `X.diagnostic()`；
 * 2. `error.rs` 里给 `AppError::diagnostic` 临时挂一个 `#[deprecated]`。
 *
 * 于是**编译器对每个站点给出两条正面输出之一**，两个方向都是「有输出」，不靠缺席推断：
 *
 * - 接收者**不是** `AppError`（没有 `diagnostic()`）⇒ `E0599: no method named \`diagnostic\``
 *   ⇒ `to_string()` 是正确用法；
 * - 接收者**是** `AppError` ⇒ `diagnostic()` 解析成功，而它此刻是 deprecated
 *   ⇒ `warning: use of deprecated method \`AppError::diagnostic\``
 *   ⇒ 该站点的 `detail` 装的是裸 code。
 *
 * ⚠️ **只注入 `.diagnostic()` 是不够的**（本脚本第一版就是这么写的，实测吃了亏）：
 * 「不报 `E0599`」是**缺席证据**，它同时被另外两类站点满足 ——
 * `#[cfg]` 掉的分支（本机不编译；实测 **13 处**，其中 7 处在 `context_menu.rs` 的
 * `#[cfg(target_os = "windows")]` 函数里，另 6 处散在 `cli.rs` / `commands/mod.rs` /
 * `commands/system_ops.rs` 的 windows / linux 分支里）与
 * **测试里 raw string 字面量中的同形串**（`tray.rs` / `tantivy_search.rs` 各 1 处）。
 * 只按 `E0599` 归组会把这两类算成 `AppError` 站点（实测 15 处「疑似」里 15 处都是假的）。
 * `#[deprecated]` 把 `AppError` 那一侧从缺席变成**正面警告**，这两类噪声自动落空。
 *
 * 副作用是好事：`diagnostic()` 在**生产代码里已有 58 个调用点**，它们会一起报
 * `use of deprecated` —— 那正是这个标记自身的**阳性对照**。口径与实测（在三棵 crate 树上跑）：
 *
 *   command grep -rn '\.diagnostic()' src-tauri/src/ src-tauri-proxy/src/ transcript-store/src/
 *     --include='*.rs' | command grep -vE ':[0-9]+:[[:space:]]{0,}/{2}' | wc -l   →  58
 *
 * （不去掉行注释是 61 行，多出的 3 行是注释里提到 `.diagnostic()` 的散文。）
 * `--report` 把「不在注入清单里的 deprecated 警告数」单独打出来，且**为 0 时拒绝出结果**：
 * 它为 0 时 `AppError` 那一侧没有任何正面信号，「桶是空的」既可能为真、也可能只是标记没生效。
 *
 * ## 射程
 *
 * 只覆盖「简单路径 + `.to_string()`」这一形状（`SITE` 正则）。跨行实参、含括号的实参
 * （`format!` / `foo().to_string()`）不在射程内。脚本每次都会把射程打印出来 ——
 * **静默跳过站点然后报「干净」正是本工具要关掉的那类缺陷**。
 *
 * ## 前置条件与还原守卫（两条路都要守）
 *
 * ⚠️ **打补丁这条路**：被跟踪文件必须没有未提交改动。还原用 `git checkout -- <file>`，
 * 它在脏树上会销毁该文件全部未提交改动（计划 12 里一个实现者踩过这个坑）。故先验，脏则 `exit 1`。
 * 判据用 `--untracked-files=no`：未跟踪文件不算脏（`git checkout --` 对它们不起作用），
 * 否则「本脚本自己还没提交」就会把这条安全条件变成一个用不了的开关。
 *
 * ⚠️ **还原这条路**：打补丁时逐文件记下**补丁后内容的 md5**（`patchedHash`）与**补丁前内容的 md5**
 * （`preHash`），`--revert` 时先比对当前内容 —— 文件的内容**既不是 `patchedHash` 也不是
 * `preHash`** 时才**拒绝还原**（不覆盖、也不静默跳过），列出它期望与实际的 md5 后 `exit 1`。
 * 认 `preHash` 是为了让「清单已落盘、文件还没轮到写」的那种半打补丁状态也还原得回来：
 * **清单在第一次写文件之前落盘**，故一次 I/O 失败或中途被 kill 不会留下「树被打过补丁、
 * 清单却不存在」的不可恢复状态。没有这条守卫，一次崩掉的运行会把树留在打过补丁的状态，
 * 之后任何人对它的编辑都会被下一次 `--revert` 的 `git checkout --` 销毁 ——
 * 与前置条件要防的是同一个坑，只是晚了一步。还原成功后还会验 `preHash` 并打印
 * `MD5_IDENTICAL_<n>_FILES`，把「树确实回到原样」这句话变成可复现的输出。
 *
 * ⚠️ **`--report` 这条路**：清单是全局的（`tmpdir()`），可能来自另一个 checkout 或另一个提交。
 * 故 `--report` 先核对 `manifest.root` 与打补丁时记下的 HEAD，并要求**每个文件都还在
 * `patchedHash` 上**（比 `--revert` 严：停在 `preHash` 的文件根本没打补丁，它的站点没有诊断，
 * 会被静默算进「编译器未给出判据」），三条任一不满足就 `exit 1`。
 *
 * ## 用法
 *
 *   node scripts/probe-detail-args.mjs                       # 打补丁 + 写清单
 *   (cd src-tauri && cargo check --workspace --all-targets > /tmp/probe.log 2>&1)
 *   node scripts/probe-detail-args.mjs --report /tmp/probe.log
 *   node scripts/probe-detail-args.mjs --revert              # 还原并验树
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/** 站点所在的三棵树。口径与 `command grep -rn '\.with("detail"' <这三棵树>` 一致。 */
export const CRATES = ["src-tauri/src", "src-tauri-proxy/src", "transcript-store/src"];
export const MANIFEST = join(tmpdir(), "probe-detail-args-manifest.json");

/** 站点开标记。这是「总站点数」的口径，**包含注释行里的同形串**（单列在 `commentOpeners`）。 */
export const OPENER = /\.with\("detail"/g;
/** 射程内的形状：`.with("detail", <简单路径>.to_string())`。路径段允许 `::`。 */
export const SITE = /(\.with\("detail",\s*)([A-Za-z_][A-Za-z0-9_:]*)(\.to_string\(\))/g;
/** 探针标记的插入点：`AppError::diagnostic` 的签名行。 */
export const MARKER_ANCHOR = /^([ \t]*)pub fn diagnostic\(&self\) -> String \{$/m;
/** 挂上 `#[deprecated]`，把「`diagnostic()` 解析成功」从沉默变成正面警告。 */
export const MARKER_REPLACEMENT = "$1#[deprecated]\n$1pub fn diagnostic(&self) -> String {";
/**
 * rustc 对 deprecated 调用的措辞，且**必须点名 `AppError::diagnostic`**。
 *
 * 只匹配「`use of deprecated`」是不够的：同一行上若另有别的 deprecated 调用，
 * 那条警告会把这个站点误判成 `AppError` 站点。实测 rustc 的原文是
 * `use of deprecated method `error::AppError::diagnostic``（模块路径随 crate 而变，故路径段用 `[^`]*` 兜住）。
 */
export const DEPRECATED = /use of deprecated method `[^`]*AppError::diagnostic`/;
/** 内容指纹：打补丁前后各记一份，`--revert` 靠它拒绝覆盖别人的改动。 */
export function md5(text) {
  return createHash("md5").update(text, "utf8").digest("hex");
}

function lineOf(source, offset) {
  let line = 1;
  for (let i = 0; i < offset; i += 1) if (source.charCodeAt(i) === 10) line += 1;
  return line;
}

/**
 * 行注释（行首可选空白 + 两个斜杠）的行号集合。
 *
 * **为什么必须排除**：注释里引用站点形状的散文（如「套一层 `.with("detail", e.to_string())`
 * 会把裸 code 塞进 detail」）与真站点逐字同形。不排除的话，注入会改写注释（编译器看不见，
 * 于是那个「站点」既不报 `E0599` 也不报 deprecated），既虚增注入数，又把它算成射程外的噪声。
 * 本仓三棵树没有 Rust 块注释（实测 1 处 `/*`，在 `session_index_watcher.rs` 的行注释散文里），
 * 故行注释判据即足够。
 *
 * ⚠️ **边界：只认行首注释**（可选空白 + `//`）。代码后跟行尾注释、且形状出现在那段注释里的行
 * （如 `foo(); // .with("detail", e.to_string())`）**会被当成站点注入** —— 注入改写的是注释文本，
 * 编译器看不见，于是它既不报 `E0599` 也不报 deprecated，只会虚增注入数。
 * 本仓今天 0 行（判据：逐行取 `//` 之后的子串，看是否含 `.with("detail"`，只看非行首注释行）。
 * 要彻底关掉得做字符串感知的行尾注释剥离，那是另一个量级的判据，故这里只具名边界。
 */
function commentLines(source) {
  const lines = new Set();
  source.split("\n").forEach((text, index) => {
    if (/^\s*\/\//.test(text)) lines.add(index + 1);
  });
  return lines;
}

/**
 * 纯函数：把射程内的站点注入 `.diagnostic()`，并把射程如实数出来。
 *
 * 返回 `{ out, sites, injected, codeOpeners, commentOpeners, outOfReach, unreached }`：
 * - `codeOpeners` —— 非注释行上的 `.with("detail"` 开标记数（**总站点数的本脚本口径**）；
 * - `injected` —— 真正被注入的站点数（= `sites.length`）；
 * - `outOfReach` —— `codeOpeners - injected`，形状不在射程内的站点数；
 * - `unreached` —— 射程外站点的 `{line, col, text}`，报告里直接列出来。
 *
 * **恒等式 `codeOpeners === injected + unreached.length` 由构造保证**：逐**开标记**判定射程，
 * 而不是逐行判定。按行判会漏掉「同一行上放了两个站点」里的第二个
 * （那一行已经有注入 ⇒ 整行被跳过），恒等式随之失效。`patch()` 会断言这条恒等式。
 */
export function injectSites(source) {
  const comments = commentLines(source);
  const all = [...source.matchAll(OPENER)];
  const codeOpeners = all.filter((m) => !comments.has(lineOf(source, m.index))).length;
  const commentOpeners = all.length - codeOpeners;

  const sites = [];
  const reachedAt = new Set();
  const out = source.replace(SITE, (match, pre, expr, _post, offset) => {
    const line = lineOf(source, offset);
    if (comments.has(line)) return match;
    reachedAt.add(offset);
    sites.push({ line, expr });
    return `${pre}${expr}.diagnostic()`;
  });

  const unreached = [];
  for (const m of all) {
    const line = lineOf(source, m.index);
    if (comments.has(line) || reachedAt.has(m.index)) continue;
    unreached.push({
      line,
      col: m.index - source.lastIndexOf("\n", m.index - 1),
      text: source.split("\n")[line - 1].trim(),
    });
  }

  return { out, sites, injected: sites.length, codeOpeners, commentOpeners, outOfReach: unreached.length, unreached };
}

/** 纯函数：给 `AppError::diagnostic` 挂 `#[deprecated]`。找不到锚点就**如实返回 false**。 */
export function markDiagnostic(source) {
  if (!MARKER_ANCHOR.test(source)) return { out: source, marked: false };
  return { out: source.replace(MARKER_ANCHOR, MARKER_REPLACEMENT), marked: true };
}

/**
 * 纯函数：两步补丁一次做完，**顺序由这里固定**。
 *
 * ⚠️ 顺序是有意义的：挂标记会在 `error.rs` 里插一行，**先注入后挂标记**会让该文件里已记录的
 * 站点行号整体下移一格，编译器报的行与清单对不上（实测踩过两次：真树上 5 处 `From` 实现落进
 * 「未给出判据」桶，夹具里阳性对照整条落空）。把两步封成一个函数，就不必靠调用方记得顺序。
 *
 * 返回的 `sites` / `unreached` 行号都是**最终源码**上的行号。
 */
export function patchSource(source) {
  const marked = markDiagnostic(source);
  return { ...injectSites(marked.out), marked: marked.marked };
}

/**
 * 纯函数：解析 `cargo check` 输出为 `{level, code, message, file, line, col}` 列表。
 *
 * 取每个诊断头（`error[E0599]: …` / `warning: …`）之后**第一条** `--> <file>:<line>:<col>`
 * （rustc 的 span 行）；其后的 `-->` 属于 note/help，不计。
 */
export function parseCargoLog(text) {
  const diags = [];
  let cur = null;
  for (const raw of text.split("\n")) {
    const head = raw.match(/^(error|warning)(?:\[([A-Z]\d+)\])?: (.*)$/);
    if (head) {
      cur = { level: head[1], code: head[2] ?? null, message: head[3] };
      continue;
    }
    const span = raw.match(/^\s*-->\s+(\S+):(\d+):(\d+)/);
    if (span && cur) {
      diags.push({ ...cur, file: span[1], line: Number(span[2]), col: Number(span[3]) });
      cur = null;
    }
  }
  return diags;
}

/**
 * 纯函数：把注入清单与编译器输出对齐，**三分类**：
 *
 * - `appError` —— 该站点报了 `use of deprecated` ⇒ 接收者是 `AppError` ⇒ `detail` 装的是裸 code；
 * - `thirdParty` —— 该站点报了 `E0599` ⇒ 接收者不是 `AppError` ⇒ `to_string()` 正确；
 * - `unjudged` —— 两者都没报 ⇒ **编译器没给出判据**（`#[cfg]` 掉的分支、字符串字面量里的
 *   同形串、或该处根本没被类型检查）。这一桶必须打印出来，**不能并进上面任何一桶**。
 *
 * `outside*` 是落在注入清单之外的同类诊断（标记自身的阳性对照 + 解析偏差），同样不静默丢弃。
 */
export function classify(sites, diags) {
  const deprecated = new Set();
  const e0599 = new Set();
  for (const d of diags) {
    const key = `${d.file}:${d.line}`;
    if (d.code === "E0599") e0599.add(key);
    else if (DEPRECATED.test(d.message)) deprecated.add(key);
  }
  const siteKeys = new Set(sites.map((s) => `${s.file}:${s.line}`));
  const bucket = (pred) => sites.filter((s) => pred(`${s.file}:${s.line}`));
  return {
    appError: bucket((k) => deprecated.has(k)),
    thirdParty: bucket((k) => !deprecated.has(k) && e0599.has(k)),
    unjudged: bucket((k) => !deprecated.has(k) && !e0599.has(k)),
    outsideDeprecated: [...deprecated].filter((k) => !siteKeys.has(k)),
    outsideE0599: [...e0599].filter((k) => !siteKeys.has(k)),
  };
}

function git(args) {
  return execFileSync("git", args, { cwd: process.cwd(), encoding: "utf8" });
}

function rustFiles() {
  return git(["ls-files", ...CRATES])
    .trim()
    .split("\n")
    .filter((f) => f.endsWith(".rs"));
}

/**
 * 前置条件：**被改动的文件必须没有未提交改动**。脏树 ⇒ 拒绝。
 *
 * 判据是 `git status --porcelain --untracked-files=no`：**未跟踪文件不算脏**。
 * 理由是这条前置条件要防的东西很具体 —— `git checkout -- <file>` 会销毁该文件全部
 * **未提交改动**，而它对未跟踪文件根本不起作用。用不带 `-uno` 的全量 `git status` 会让
 * 「本脚本自己还没提交」这种情形也触发拒绝，把一个安全条件变成一个用不了的开关。
 * 已暂存与未暂存的改动都会被 `--porcelain` 报出，两者都会挡住打补丁。
 */
function assertClean() {
  const dirty = git(["status", "--porcelain", "--untracked-files=no"]).trim();
  if (dirty) {
    console.error("⛔ 被跟踪文件有未提交改动，拒绝就地打补丁（`git checkout --` 会销毁它们）：\n" + dirty);
    process.exit(1);
  }
}

function patch() {
  assertClean();
  const errorRs = "src-tauri/src/error.rs";

  // 先把每个文件的补丁在内存里算完、把守卫全跑完，**再动磁盘**：任何一条守卫失败时
  // 树都还没被改过，不存在「失败了但树已经半打补丁」的中间态。
  //
  // ⚠️ 这条只覆盖**守卫失败**这一种情形。I/O 失败、进程被 kill 这类**在写文件途中**
  // 发生的事故，守卫拦不到 —— 所以清单必须在**第一次写文件之前**就落盘（见下）。
  const results = rustFiles().map((file) => {
    const src = readFileSync(file, "utf8");
    // `error.rs` 多挂一个探针标记（把 `AppError` 那一侧从「沉默」变成「正面警告」）；
    // 顺序由 `patchSource` 固定 —— 挂标记会插一行，先注入后挂标记会让行号对不上。
    const r = file === errorRs ? patchSource(src) : injectSites(src);
    return { file, src, r };
  });

  const marker = results.find((x) => x.file === errorRs);
  if (!marker?.r.marked) {
    console.error(`⛔ 在 ${errorRs} 里找不到 \`pub fn diagnostic(&self) -> String {\` 锚点，拒绝出一个判据残缺的结果。`);
    process.exit(1);
  }

  const codeOpeners = results.reduce((n, x) => n + x.r.codeOpeners, 0);
  const commentOpeners = results.reduce((n, x) => n + x.r.commentOpeners, 0);
  const injected = results.reduce((n, x) => n + x.r.injected, 0);
  const unreached = results.flatMap((x) => x.r.unreached.map((u) => ({ file: x.file, line: u.line, col: u.col, text: u.text })));
  // 恒等式：总站点 = 注入 + 射程外。它由 `injectSites` 的逐开标记判据保证，
  // 但**必须在这里断言** —— 射程对不上账时，「还有几个没看」就没有下界，
  // 而那正是「静默跳过站点然后报干净」的入口。
  if (codeOpeners !== injected + unreached.length) {
    console.error(`⛔ 射程账目不平：总站点 ${codeOpeners} ≠ 注入 ${injected} + 射程外 ${unreached.length}。拒绝出一个对不上账的射程。`);
    process.exit(1);
  }

  const touched = [];
  const sites = [];
  const preHash = {};
  const patchedHash = {};
  const pending = [];
  for (const { file, src, r } of results) {
    if (r.out === src) continue;
    // 前后两份 md5 都**在写盘之前**算得出来（`r.out` 已经在内存里），所以清单可以先落盘。
    preHash[file] = md5(src);
    patchedHash[file] = md5(r.out);
    touched.push(file);
    pending.push({ file, out: r.out });
    for (const s of r.sites) sites.push({ file, line: s.line, expr: s.expr });
  }

  // ⚠️ **清单必须先于第一次写文件落盘**。反过来（改完文件再写清单）时，一次 I/O 失败
  // 或中途被 kill 会把树留在打过补丁的状态而**没有清单** —— 没有清单就没有
  // `preHash` / `patchedHash`，`--revert` 连比对的基准都没有，那份补丁再也还原不回去。
  // 先落盘后写文件，最坏情况是「清单说 A、B 都打过补丁，实际只打了 A」；`revert()`
  // 认 `patchedHash` 与 `preHash` 两种内容，故这两种状态都还原得回来。
  writeFileSync(
    MANIFEST,
    JSON.stringify(
      {
        root: process.cwd(),
        head: git(["rev-parse", "HEAD"]).trim(),
        files: touched,
        preHash,
        patchedHash,
        sites,
        codeOpeners,
        commentOpeners,
        unreached,
      },
      null,
      2
    )
  );

  for (const { file, out } of pending) writeFileSync(file, out);
  console.log(`总站点（非注释行的 \`.with("detail"\` 开标记）: ${codeOpeners}`);
  console.log(`  其中注释行上的同形串（不计入）: ${commentOpeners}`);
  console.log(`射程内已注入: ${injected}（跨 ${touched.length} 个文件，含挂标记的 ${errorRs}）`);
  console.log(`射程外（跨行 / 含括号 / 其它形状）: ${unreached.length}`);
  console.log(`账目: ${codeOpeners} − ${injected} = ${unreached.length} ✅`);
  console.log(`清单: ${MANIFEST}`);
  if (unreached.length) {
    console.log("\n射程外站点：");
    for (const u of unreached) console.log(`  ${u.file}:${u.line}:${u.col}  ${u.text}`);
  }
}

function report(logPath) {
  const manifest = JSON.parse(readFileSync(MANIFEST, "utf8"));
  // 清单是全局的（`tmpdir()`），可能来自**另一个 checkout 或另一个提交**。
  // 不核对就出报告，等于拿 A 树的站点清单去对 B 树的编译日志：路径对不上时站点会整片
  // 落进「编译器未给出判据」，而输出里看不出这是清单错了 —— 又是一个「对不上账却看不出来」。
  if (manifest.root !== process.cwd()) {
    console.error(`⛔ 清单来自另一个 checkout：清单 root=${manifest.root}，当前 cwd=${process.cwd()}。`);
    console.error("   拒绝用别处的站点清单去对这里的编译日志。请先在当前树重跑一次打补丁。");
    process.exit(1);
  }
  const head = git(["rev-parse", "HEAD"]).trim();
  if (manifest.head !== head) {
    console.error(`⛔ 清单已过期：打补丁时 HEAD=${manifest.head ?? "<清单里没有记录>"}，当前 HEAD=${head}。`);
    console.error("   这期间的提交可能动过站点，清单里的行号与内容都不再是当前树的。请重跑打补丁。");
    process.exit(1);
  }
  // 清单落盘早于写文件（见 `patch()`），所以这里要比 `revert()` 严：报告要求**每个文件
  // 都还在打过补丁的内容上**。某个文件停在 `preHash` 说明它根本没被打补丁，
  // 它的站点不会有诊断，会被静默算进「编译器未给出判据」。
  const notPatched = manifest.files.filter((f) => {
    try {
      return md5(readFileSync(f, "utf8")) !== manifest.patchedHash[f];
    } catch {
      return true;
    }
  });
  if (notPatched.length) {
    console.error(`⛔ 清单里的文件不在打过补丁的内容上（日志未必来自这棵树）：\n${notPatched.join("\n")}`);
    console.error("   拒绝出一个对不上账的结论。请重跑打补丁 + cargo check。");
    process.exit(1);
  }

  const diags = parseCargoLog(readFileSync(logPath, "utf8"));
  const { appError, thirdParty, unjudged, outsideDeprecated, outsideE0599 } = classify(manifest.sites, diags);
  const codes = diags.reduce((acc, d) => {
    const k = d.code ?? `${d.level}: ${d.message.slice(0, 40)}`;
    acc[k] = (acc[k] ?? 0) + 1;
    return acc;
  }, {});
  console.log(`注入站点: ${manifest.sites.length}（射程外 ${manifest.unreached.length}）`);
  console.log(`cargo 诊断分布: ${JSON.stringify(codes)}`);
  console.log(
    `三分类: AppError ${appError.length} / 第三方 ${thirdParty.length} / 编译器未给出判据 ${unjudged.length}`
  );
  console.log(
    `清单外的 deprecated 警告（标记自身的阳性对照，应为生产代码里已知的 \`.diagnostic()\` 调用点）: ${outsideDeprecated.length}`
  );
  console.log(`清单外的 E0599: ${outsideE0599.length}`);

  // 标记自身的阳性对照**必须非零**：它为 0 时，`AppError` 那一侧就没有任何正面信号，
  // 「AppError 桶是空的」这句话既可能为真、也可能只是标记没生效 —— 两者在输出上无法区分。
  // 这正是本工具存在的理由（判据要有活性证明），所以这里不是打印一行就算，而是**拒绝出结果**。
  if (outsideDeprecated.length === 0) {
    console.error("⛔ 清单外的 deprecated 警告为 0：探针标记没有生效（或 `diagnostic()` 的生产调用点全没了）。");
    console.error("  这条是标记自身的阳性对照。它为 0 时「AppError 桶为空」没有活性证明，拒绝出一个无对照的结果。");
    process.exit(1);
  }

  console.log(`\n== detail 收 AppError ⇒ 装的是裸 code，要修（${appError.length} 处）==`);
  for (const s of appError) console.log(`  ${s.file}:${s.line}  ${s.expr}`);
  console.log(`\n== detail 收第三方错误 ⇒ to_string() 正确，不改（${thirdParty.length} 处）==`);
  for (const s of thirdParty) console.log(`  ${s.file}:${s.line}  ${s.expr}`);
  console.log(`\n== 编译器未给出判据（cfg 掉的分支 / 字面量 / 未被类型检查）⇒ 人工看（${unjudged.length} 处）==`);
  for (const s of unjudged) console.log(`  ${s.file}:${s.line}  ${s.expr}`);
  if (outsideDeprecated.length) {
    console.log(`\n清单外的 deprecated 警告（${outsideDeprecated.length} 处）：`);
    for (const k of outsideDeprecated) console.log(`  ${k}`);
  }
}

/**
 * 纯函数：清单里哪些文件的**当前内容**与打补丁时写下的不一致（读不到也算不一致）。
 *
 * 这是 `--revert` 的守卫。`assertClean` 守的是**打补丁**那条路，还原那条路原先没有任何守卫：
 * 打完补丁之后若有人改了某个文件（一次崩掉的运行把树留在打过补丁的状态、之后有人继续编辑、
 * 或拿一份过期清单去还原），`git checkout --` 会把那份改动**连同补丁一起销毁**——
 * 与前置条件要防的是同一个坑，只是晚了一步。
 *
 * ⚠️ **认两种内容，不是一种**：`patchedHash`（已打补丁，要还原）与 `preHash`
 * （清单已落盘但该文件还没轮到写，或写之前就崩了）。只认 `patchedHash` 的话，
 * 一次中途崩掉的运行会留下一份「部分文件对得上、部分对不上」的清单，
 * `--revert` 于是**永远拒绝还原** —— 正是本项要修的那个不可恢复状态。
 * 两种都不是才算「被人改过」，才拒绝。
 */
export function tamperedFiles(manifest, read = (f) => readFileSync(f, "utf8")) {
  return manifest.files.filter((f) => {
    let current = null;
    try {
      current = read(f);
    } catch {
      current = null;
    }
    if (current === null) return true;
    const hash = md5(current);
    return hash !== manifest.patchedHash[f] && hash !== manifest.preHash?.[f];
  });
}

function revert() {
  const manifest = JSON.parse(readFileSync(MANIFEST, "utf8"));
  const tampered = tamperedFiles(manifest);
  if (tampered.length) {
    console.error("⛔ 拒绝还原：以下文件的内容与打补丁时写下的不一致（打补丁之后被改过）：");
    for (const f of tampered) {
      let current = "<读不到>";
      try {
        current = md5(readFileSync(f, "utf8"));
      } catch {}
      console.error(`  ${f}\n    打补丁时 md5=${manifest.patchedHash[f]}  现在 md5=${current}`);
    }
    console.error("这些改动可能是人的工作，`git checkout --` 会连它们一起销毁。");
    console.error("脚本既不覆盖、也不静默跳过：请先自行处置（把改动移走或提交），再重跑 --revert。");
    process.exit(1);
  }

  if (manifest.files.length) execFileSync("git", ["checkout", "--", ...manifest.files], { cwd: process.cwd() });

  const unrestored = manifest.files.filter((f) => {
    try {
      return md5(readFileSync(f, "utf8")) !== manifest.preHash[f];
    } catch {
      return true;
    }
  });
  const dirty = git(["status", "--porcelain", "--untracked-files=no"]).trim();
  console.log(`已还原 ${manifest.files.length} 个文件。`);
  if (unrestored.length) {
    console.error(`⛔ 还原后内容与打补丁前不一致：\n${unrestored.join("\n")}`);
    process.exit(1);
  }
  console.log(`md5 与打补丁前逐一相同: ${manifest.files.length}/${manifest.files.length}`);
  console.log(dirty ? `⛔ 被跟踪文件仍有改动：\n${dirty}` : "✅ git status --porcelain --untracked-files=no 为空");
  if (!dirty) console.log(`MD5_IDENTICAL_${manifest.files.length}_FILES`);
  process.exit(dirty ? 1 : 0);
}

/**
 * 是不是被当成命令直接跑的（而不是被 import 进测试）。
 *
 * ⚠️ **必须比 realpath，不能比字面路径**：从符号链接下跑（如 macOS 的 `/tmp` → `/private/tmp`，
 * 或任何 `ln -s` 出来的副本）时，`import.meta.url` 与 `argv[1]` 字面不等，判据会变成 false ——
 * 脚本于是**什么都不做、还退出 0**。在一个以「关掉静默跳过」为存在理由的工具里，
 * 那正是它自己要关掉的那类缺陷（评审从 `/tmp` 跑副本时撞上过，一度误当成通过）。
 */
const invokedDirectly = (() => {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (invokedDirectly) {
  const [mode, arg] = process.argv.slice(2);
  if (mode === "--report") {
    if (!arg) throw new Error("--report 需要 cargo 日志路径");
    report(arg);
  } else if (mode === "--revert") {
    revert();
  } else if (mode === undefined) {
    patch();
  } else {
    console.error("用法: probe-detail-args.mjs [--report <cargo.log> | --revert]");
    process.exit(2);
  }
}
