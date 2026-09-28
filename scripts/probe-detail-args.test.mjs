import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
  DEPRECATED,
  classify,
  injectSites,
  markDiagnostic,
  md5,
  parseCargoLog,
  patchSource,
  tamperedFiles,
} from "./probe-detail-args.mjs";

/**
 * 探针的判据是**编译器说的**，所以阳性/阴性对照也必须是**真的编译器输出**，
 * 不能在夹具里手写一份「编译器大概会这么说」的日志 —— 那样测的只是 `classify` 的分支，
 * 而「`AppError` 会报 deprecated、`io::Error` 会报 E0599」这个前提本身没人验。
 * 本文件因此真的调 `rustc` 编一份无依赖夹具（不建 cargo 工程、不碰 target/，约 1 秒）。
 *
 * ⚠️ **依赖声明**：`rustc` 必须在 `PATH` 上。**没有任何 CI job 安装 Rust**，但这**不等于**
 * 「没有 CI job 会跑它」—— `frontend` job 跑在 GitHub 的 ubuntu runner 镜像上，**镜像自带 `rustc`**
 * （判据：镜像公布的已装软件清单里列有 `Rust 1.98.1`，2026-09-27 读到），故 `hasRustc` 在那个 job 里
 * 为真、`it.skipIf(!hasRustc)` 不跳过：**该 job 一跑，这组对照就会执行**。
 * 「会不会被跳过」取决于 runner 镜像带不带 `rustc`，不取决于 `ci.yml` 里装了什么。
 * 实测 `.github/workflows/ci.yml` 的三个 job：`npm test` 只出现在 `frontend` job 里，而那个 job
 * 的 `uses:` 只有 `actions/setup-node@v4`（`npm ci` + `npm run build:web` + `npm test`）；
 * `backend` 与 `windows` 都装 Rust（`dtolnay/rust-toolchain@stable`）、都跑 `npm run build:proxy`，
 * 但**都不跑 `npm test`** —— `backend` 跑 `cargo check --workspace --all-targets` +
 * `cargo test --workspace`，`windows` **只跑 `cargo check`**（不跑 `cargo test`）。
 * 缺 rustc 时这组对照**跳过**并把原因打在 stderr 与套件名上，而不是静默通过。
 * 要让它在「镜像不带 `rustc`」时也稳定执行，得给 `frontend` job 加一步 `install Rust`，
 * 或让 `backend` job 跑 `npm test` —— **两者都尚未存在**，是否这么做不在本任务范围内。
 * 判定「跳过」的探测在下面 `hasRustc` 处。
 *
 * ⚠️ 夹具里的 `AppError` 与仓库里那个**逐字同形**到探针需要的程度：`Display` 是裸 code、
 * 有 `pub fn diagnostic(&self) -> String`。探针注入的是 `.diagnostic()`，判据落在这个签名上；
 * 夹具若把签名写歪，`markDiagnostic` 会返回 `marked: false`，脚本在真树上会直接拒绝出结果。
 */

const tempRoots = [];
afterAll(() => {
  for (const root of tempRoots) rmSync(root, { recursive: true, force: true });
});

/** 阳性/阴性对照要真的调 rustc；缺了就跳过并说明，见文件头。 */
const hasRustc = (() => {
  const probe = spawnSync("rustc", ["--version"], { encoding: "utf8" });
  return !probe.error && probe.status === 0;
})();
/** 跳过的理由要出现在两个地方：跑测试的人看得见的**套件名**，以及 stderr。 */
const RUSTC_MISSING = "（已跳过：PATH 上没有 rustc —— 本组对照的判据必须来自真的编译器）";
if (!hasRustc) {
  // 走 stderr 而不是 console.warn：vitest 会吞掉模块加载期的 console 输出，警告会看不见。
  process.stderr.write(
    "[probe-detail-args] 跳过「阳性/阴性对照」：PATH 上没有 rustc。\n" +
      "  这组对照的判据必须来自真的编译器（手写日志会让判据与被判对象同源），故它需要 rustc。\n" +
      "  ⚠️ 没有任何 CI job **安装** Rust；它会不会在 frontend job 里跑，取决于 runner 镜像是否自带 rustc\n" +
      "  （镜像自带时 hasRustc 为真，it.skipIf 不跳过，这组对照就会执行）。npm test 只在 frontend job 里跑，\n" +
      "  那个 job 不装 Rust；backend / windows 都装 Rust、都跑 npm run build:proxy，但都不跑 npm test\n" +
      "  （backend 跑 cargo check + cargo test，windows 只跑 cargo check）。\n"
  );
}

const FIXTURE = `use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone)]
pub enum AppError {
    Coded { code: &'static str, params: BTreeMap<String, String> },
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Coded { code, .. } => write!(f, "{}", code),
        }
    }
}

impl std::error::Error for AppError {}

impl AppError {
    pub fn coded(code: &'static str) -> Self {
        AppError::Coded { code, params: BTreeMap::new() }
    }

    pub fn with<K: Into<String>, V: Into<String>>(self, key: K, value: V) -> Self {
        match self {
            AppError::Coded { code, mut params } => {
                params.insert(key.into(), value.into());
                AppError::Coded { code, params }
            }
        }
    }

    pub fn diagnostic(&self) -> String {
        match self {
            AppError::Coded { code, params } => {
                let mut out = String::from(*code);
                for (key, value) in params {
                    out.push(' ');
                    out.push_str(key);
                    out.push('=');
                    out.push_str(value);
                }
                out
            }
        }
    }
}

fn inner_app_error() -> Result<(), AppError> {
    Err(AppError::coded("inner.code"))
}

pub fn positive_control() -> Result<(), AppError> {
    inner_app_error().map_err(|e| AppError::coded("wrap.positive").with("detail", e.to_string()))
}

pub fn negative_control() -> Result<(), AppError> {
    std::fs::read("/definitely/not/here")
        .map(|_| ())
        .map_err(|e| AppError::coded("wrap.negative").with("detail", e.to_string()))
}
`;

/** 期望行号必须从**打补丁后**的源码上取：探针标记会插一行，夹具原文的行号是旧的。 */
function lineWith(source, needle) {
  const index = source.split("\n").findIndex((line) => line.includes(needle));
  expect(index, `夹具里找不到 ${needle}`).toBeGreaterThanOrEqual(0);
  return index + 1;
}

/** 跑真的 rustc，返回它**实际**吐出的诊断。 */
function compileFixture() {
  const root = mkdtempSync(join(tmpdir(), "probe-detail-args-"));
  tempRoots.push(root);
  const patched = patchSource(FIXTURE);
  expect(patched.marked, "夹具的 diagnostic 签名必须能被探针标记挂上").toBe(true);
  const source = join(root, "fixture.rs");
  writeFileSync(source, patched.out);
  const r = spawnSync(
    "rustc",
    ["--edition", "2021", "--crate-type", "lib", "--emit", "metadata", "-o", join(root, "fixture.rmeta"), source],
    { encoding: "utf8" }
  );
  if (r.error) throw new Error(`本测试要真的调 rustc：${r.error.message}`);
  return { patched, source, status: r.status, diags: parseCargoLog(`${r.stdout ?? ""}\n${r.stderr ?? ""}`) };
}

describe("探针的射程与注入", () => {
  it("只注入「简单路径 + .to_string()」这一形状，并把射程外如实列出来", () => {
    const src = [
      'AppError::coded("a.b").with("detail", e.to_string())',
      'AppError::coded("a.b").with("detail", format!("{:?}", e))',
      'AppError::coded("a.b").with("detail", path)',
      'AppError::coded("a.b").with("detail", e)',
    ].join("\n");
    const r = injectSites(src);
    expect(r.injected).toBe(1);
    expect(r.codeOpeners).toBe(4);
    expect(r.outOfReach).toBe(3);
    expect(r.out).toContain("e.diagnostic()");
    // 射程外的三种形状原样留着：一个都没被改写
    expect(r.out).toContain('format!("{:?}", e)');
    expect(r.out).toContain('.with("detail", path)');
    // 账要平：总站点 = 注入 + 射程外
    expect(r.injected + r.outOfReach).toBe(r.codeOpeners);
  });

  it("注释行上的同形串既不算站点、也不注入", () => {
    const src = [
      '// 套一层 `.with("detail", e.to_string())` 会把裸 code 塞进 detail',
      'AppError::coded("a.b").with("detail", e.to_string())',
    ].join("\n");
    const r = injectSites(src);
    expect(r.injected).toBe(1);
    expect(r.commentOpeners).toBe(1);
    expect(r.codeOpeners).toBe(1);
    expect(r.out).toContain('// 套一层 `.with("detail", e.to_string())` 会把裸 code 塞进 detail');
  });

  it("同一行上放两个站点时账仍然平：射程按**开标记**判，不按行判", () => {
    // 按行判会漏掉第二个（那一行已经有注入 ⇒ 整行被跳过），恒等式随之失效。
    const src = 'let a = x.with("detail", e.to_string()); let b = y.with("detail", f);';
    const r = injectSites(src);
    expect(r.codeOpeners).toBe(2);
    expect(r.injected).toBe(1);
    expect(r.outOfReach).toBe(1);
    expect(r.injected + r.outOfReach).toBe(r.codeOpeners);
    expect(r.unreached).toHaveLength(1);
    expect(r.unreached[0].col).toBe(src.lastIndexOf('.with("detail"') + 1);
  });

  it("锚点缺失时如实返回 marked: false，不静默出一个判据残缺的结果", () => {
    expect(markDiagnostic("pub fn diagnostics(&self) -> String {").marked).toBe(false);
    expect(markDiagnostic("pub fn diagnostic(&self) -> String {").marked).toBe(true);
    expect(markDiagnostic("    pub fn diagnostic(&self) -> String {").marked).toBe(true);
  });
});

describe(`阳性/阴性对照：判据由真的 rustc 给出${hasRustc ? "" : RUSTC_MISSING}`, () => {
  it.skipIf(!hasRustc)("detail 收 AppError 的站点被判为 AppError；收 io::Error 的被判为第三方", () => {
    const { patched, source, status, diags } = compileFixture();

    // 前提一：阴性对照真的让编译器报了错（否则「没报」可能只是注入没生效）
    expect(status, "夹具里有 E0599，rustc 必须非 0 退出").not.toBe(0);
    expect(diags.some((d) => d.code === "E0599")).toBe(true);
    // 前提二：阳性对照真的让编译器报了 deprecated（标记失灵时这条先红）。
    // 这里用的是分类器**同一份**判据 `DEPRECATED`，顺带验它认得 rustc 的真实措辞。
    expect(diags.some((d) => DEPRECATED.test(d.message))).toBe(true);

    // `injectSites` 只给出行号；`file` 由清单装配时补上（真树上是相对仓库根的路径）。
    const sites = patched.sites.map((s) => ({ ...s, file: source }));
    const { appError, thirdParty, unjudged } = classify(sites, diags);
    const positive = lineWith(patched.out, 'AppError::coded("wrap.positive")');
    const negative = lineWith(patched.out, 'AppError::coded("wrap.negative")');

    expect(appError.map((s) => s.line)).toEqual([positive]);
    expect(thirdParty.map((s) => s.line)).toEqual([negative]);
    expect(unjudged).toEqual([]);
  });

  it.skipIf(!hasRustc)("两个站点被注入的形态相同，差别只在接收者的类型上", () => {
    const { patched } = compileFixture();
    expect(patched.injected).toBe(2);
    expect(patched.sites.map((s) => s.expr)).toEqual(["e", "e"]);
  });
});

describe("三分类不许把「编译器没说话」并进 AppError", () => {
  const sites = [
    { file: "src/x.rs", line: 10, expr: "e" },
    { file: "src/x.rs", line: 20, expr: "e" },
    { file: "src/x.rs", line: 30, expr: "e" },
  ];
  const log = [
    "warning: use of deprecated method `AppError::diagnostic`",
    "  --> src/x.rs:10:20",
    "error[E0599]: no method named `diagnostic` found for struct `io::Error` in the current scope",
    "  --> src/x.rs:20:20",
  ].join("\n");

  it("报 deprecated 的进 AppError，报 E0599 的进第三方，两者都没报的进 unjudged", () => {
    const { appError, thirdParty, unjudged, outsideDeprecated, outsideE0599 } = classify(sites, parseCargoLog(log));
    expect(appError.map((s) => s.line)).toEqual([10]);
    expect(thirdParty.map((s) => s.line)).toEqual([20]);
    expect(unjudged.map((s) => s.line)).toEqual([30]);
    expect(outsideDeprecated).toEqual([]);
    expect(outsideE0599).toEqual([]);
  });

  it("清单外的诊断不静默丢弃：标记自身的阳性对照要能数出来", () => {
    const extra = `${log}\nwarning: use of deprecated method \`AppError::diagnostic\`\n  --> src/other.rs:7:3`;
    const { outsideDeprecated } = classify(sites, parseCargoLog(extra));
    expect(outsideDeprecated).toEqual(["src/other.rs:7"]);
  });

  it("判据必须点名 AppError::diagnostic：同一行上别的 deprecated 调用不算数", () => {
    // 只匹配「use of deprecated」的话，下面这条会把 :10 误判成 AppError 站点。
    const unrelated = [
      "warning: use of deprecated method `std::fs::read_dir`",
      "  --> src/x.rs:10:20",
    ].join("\n");
    const { appError, unjudged } = classify(sites, parseCargoLog(unrelated));
    expect(appError).toEqual([]);
    expect(unjudged.map((s) => s.line)).toEqual([10, 20, 30]);
  });

  it("判据认得 rustc 的真实措辞（模块路径随 crate 而变，仍须点名 AppError::diagnostic）", () => {
    expect(DEPRECATED.test("use of deprecated method `error::AppError::diagnostic`")).toBe(true);
    expect(DEPRECATED.test("use of deprecated method `AppError::diagnostic`")).toBe(true);
    expect(DEPRECATED.test("use of deprecated method `std::fs::read_dir`")).toBe(false);
  });
});

describe("--revert 的守卫：内容对不上就拒绝还原", () => {
  const manifest = {
    files: ["a.rs", "b.rs"],
    patchedHash: { "a.rs": md5("patched-a"), "b.rs": md5("patched-b") },
  };
  const readFrom = (contents) => (f) => {
    if (!(f in contents)) throw new Error(`ENOENT: ${f}`);
    return contents[f];
  };

  it("全部匹配时放行", () => {
    expect(tamperedFiles(manifest, readFrom({ "a.rs": "patched-a", "b.rs": "patched-b" }))).toEqual([]);
  });

  it("打补丁之后被改过的文件要被报出来（它可能承载着人的工作）", () => {
    expect(tamperedFiles(manifest, readFrom({ "a.rs": "patched-a", "b.rs": "有人改过" }))).toEqual(["b.rs"]);
  });

  it("文件读不到（被删 / 被移走）也算不一致，不当作「无事发生」", () => {
    expect(tamperedFiles(manifest, readFrom({}))).toEqual(["a.rs", "b.rs"]);
  });

  it("停在 preHash 的文件也放行：清单先落盘、文件还没轮到写的那种半打补丁状态要还原得回来", () => {
    // 清单在第一次写文件之前落盘（见 patch()），故「部分文件已打补丁、部分还是原样」
    // 是正常的事故残留。只认 patchedHash 会让它永远拒绝还原。
    const withPre = {
      files: ["a.rs", "b.rs"],
      patchedHash: { "a.rs": md5("patched-a"), "b.rs": md5("patched-b") },
      preHash: { "a.rs": md5("pre-a"), "b.rs": md5("pre-b") },
    };
    expect(tamperedFiles(withPre, readFrom({ "a.rs": "patched-a", "b.rs": "pre-b" }))).toEqual([]);
    // 两种内容都不是时照旧拒绝：这是「有人改过」，`git checkout --` 会连它一起销毁。
    expect(tamperedFiles(withPre, readFrom({ "a.rs": "patched-a", "b.rs": "有人改过" }))).toEqual(["b.rs"]);
  });
});
