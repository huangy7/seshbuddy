import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import {
  rustCliIds,
  rustEnumVariants,
  rustCapability,
  tsConstArrayMembers,
  tsUnionMembers,
} from "./enumParity";
import { CLI_DEFINITIONS, CLI_IDS, type CliId } from "../types/cli";

// 不写成 `new URL("../../…", import.meta.url)`：那个字面形状是 Vite 的资源引用语法，
// 会被改写成非 file: 的模块 URL，fileURLToPath 直接抛「The URL must be of scheme file」。
// 经一个变量转手可避开该改写，实测取到的是本文件真实路径。
const TEST_FILE_URL = import.meta.url;
const REPO_ROOT = join(fileURLToPath(new URL("../../", TEST_FILE_URL)));
const read = (...parts: string[]) => readFileSync(join(REPO_ROOT, ...parts), "utf8");

/**
 * 手写的 Rust↔TS 镜像枚举，逐个配 parity 守卫。
 *
 * `MatchedField` 的那一份在 `./matchedField.parity.test.ts`（它另有行为侧断言）。
 * 本文件收的是同批迁移新造、当时漏了守卫的两个。
 *
 * 为什么值得单独钉：Rust 加一个变体而 TS 联合没跟上时，**联合本身没变**，
 * `vue-tsc` 不响，运行时才落进 `default` / 返回 `undefined` 并上屏。
 */
const MIRRORS = [
  {
    name: "ExportOutcome",
    rust: ["src-tauri", "src", "commands", "session.rs"],
    ts: ["src", "types", "session.ts"],
    why: "`exportOutcomeText` 的 switch 落空会让 `message(undefined)` 上屏",
  },
  {
    name: "TrafficSessionLabel",
    rust: ["src-tauri", "src", "proxy.rs"],
    ts: ["src", "composables", "useProxy.ts"],
    why: "`sessionLabel` 的 switch 落空会让流量面板的行标题变空",
  },
];

describe("Rust↔TS 镜像枚举的 parity", () => {
  /**
   * 提取器自身的阳性/阴性对照。
   *
   * ⚠️ 没有它，「两边一致」与「两边都抽到空集」不可区分 —— 一个匹配不到东西的正则
   * 会让下面每条 parity 恒绿。
   */
  it("提取器在含枚举/联合的文本上报得出，在无关文本上返回空", () => {
    // 单元变体与结构体变体都要认得（本仓两种都有）
    expect(rustEnumVariants("pub enum E {\n    Single,\n    Merged { count: usize },\n}", "E"))
      .toEqual(["single", "merged"]);
    expect(rustEnumVariants("没有枚举的文本", "E")).toEqual([]);
    expect(rustEnumVariants("pub enum E {\n    Single,\n}", "别的名字")).toEqual([]);

    // 字符串联合与带判别式的 tagged union 都要认得
    expect(tsUnionMembers('export type T = "a" | "b";', "T")).toEqual(["a", "b"]);
    // ⚠️ tagged union 的对象成员里**有分号**（`{ kind: "b"; n: number }`）：
    // 若用 `[^;]+` 取类型体，会在第一个分号处截断，静默漏掉后面的成员。
    expect(tsUnionMembers('export type T =\n  | { kind: "a" }\n  | { kind: "b"; n: number };', "T"))
      .toEqual(["a", "b"]);
    expect(tsUnionMembers("没有联合的文本", "T")).toEqual([]);

    // `CliKind::id()` 的字面量：抽的是 `=> "…"` 的取值，不是变体名
    expect(rustCliIds('pub fn id(self) -> &\'static str {\n    match self {\n        Self::WorkBuddy => "workbuddy",\n    }\n}'))
      .toEqual(["workbuddy"]);
    expect(rustCliIds("没有 id 函数的文本")).toEqual([]);

    // 收窄派生的清单（`as const` 数组）—— `tsUnionMembers` 抽不到这种形态
    expect(tsConstArrayMembers('export const IDS = ["a", "b"] as const;', "IDS")).toEqual(["a", "b"]);
    expect(tsConstArrayMembers("没有该常量的文本", "IDS")).toEqual([]);

    // 能力位：Option 访问器看 Some/None，布尔访问器看 true/false
    expect(rustCapability("fn new_session(&self) -> Option<X> {\n    Some(&X)\n}", "new_session")).toBe(true);
    expect(rustCapability("fn fork(&self) -> Option<X> {\n    None\n}", "fork")).toBe(false);
    expect(rustCapability("fn can_delete(&self) -> bool {\n    false\n}", "can_delete")).toBe(false);
    expect(rustCapability("fn can_delete(&self) -> bool {\n    true\n}", "can_delete")).toBe(true);
    expect(rustCapability("没有该方法的文本", "can_delete")).toBeUndefined();
  });

  /**
   * `CliKind` ↔ `CLI_IDS`：前端唯一的手写清单与 Rust 的线上 id 必须一致。
   *
   * 判据取自 `CliKind::id()` 而**非**枚举变体名 —— 变体名与线上 id 不是同一条映射
   * （`WorkBuddy` 的 id 是 `workbuddy`），按变体名折算会在两边都「看起来对」时静默判错。
   */
  it("CliKind 的 id 串与前端 CLI_IDS 相同", () => {
    const rust = rustCliIds(read("src-tauri", "src", "cli.rs"));
    const ts = tsConstArrayMembers(read("src", "types", "cli.ts"), "CLI_IDS");

    expect(rust, "cli.rs 里没抽到 CliKind::id() 的任何 id").not.toEqual([]);
    expect(ts, "cli.ts 里没抽到 CLI_IDS 的任何成员").not.toEqual([]);
    expect(ts, "Rust 加了 CLI 而前端 CLI_IDS 没跟上：该 CLI 的会话在前端整体不可见").toEqual(rust);
  });

  for (const mirror of MIRRORS) {
    it(`${mirror.name}：Rust 变体集与 TS 联合成员集相同`, () => {
      const rust = rustEnumVariants(read(...mirror.rust), mirror.name);
      const ts = tsUnionMembers(read(...mirror.ts), mirror.name);

      expect(rust, `${mirror.rust.join("/")} 里没抽到 ${mirror.name} 的任何变体`).not.toEqual([]);
      expect(ts, `${mirror.ts.join("/")} 里没抽到 ${mirror.name} 的任何成员`).not.toEqual([]);
      expect(ts, `Rust 加了变体而 TS 联合没跟上：${mirror.why}`).toEqual(rust);
    });
  }

  /**
   * 前端能力位 → Rust 源上的判据方法。判据是方法体表达的能力有无：
   * `Option` 访问器看 `Some(...)`/`None`，布尔访问器（`can_delete`）看 `true`/`false`。
   */
  const CAPABILITY_METHODS: Record<string, string> = {
    supportsNewSession: "new_session",
    supportsResumeSession: "resume_session",
    supportsInPlaceFork: "fork",
    supportsUsageStats: "usage_stats",
    supportsApiProfiles: "api_profile",
    supportsApiLogs: "api_proxy",
    supportsDelete: "can_delete",
  };

  /**
   * `supportsContextMenu` 刻意不配 Rust 判据：它是**纯前端关切**。系统右键菜单只有
   * Claude 一条（`context_menu.rs` 里硬编码 "claude"），不由 `CliSource` 回答，
   * Rust 侧没有可派生该位的源。此处显式列名，而不是悄悄从表里漏掉。
   */
  const FRONTEND_ONLY_BITS = ["supportsContextMenu"] as const;

  /**
   * `CliSource` 的能力位 ↔ 前端 `CLI_DEFINITIONS` 的 `supports*`。
   *
   * **为什么必须有**：上面那条 `CliKind::id()` ↔ `CLI_IDS` 只钉住 id 集合，**钉不住能力位**。
   * 位对不上时前端照常编译、全部测试照常绿，运行时才「界面提供了后端不做的入口」
   * 或「后端支持而界面藏起来」—— 后者正是库型源删除静默报成功那一类缺陷。
   *
   * 判据从 Rust 源派生：`sources/<id>.rs` 的 `impl CliSource` 里各能力访问器的方法体。
   * 文件名即 `CliKind::id()` 的串（`sources/mod.rs` 的 `mod <id>;` 与 `id()` 同串）。
   */
  it("CliSource 的能力位与前端 CLI_DEFINITIONS 逐 CLI 一致", () => {
    const sourcesDir = join(REPO_ROOT, "src-tauri", "src", "cli_registry", "sources");
    const ids = readdirSync(sourcesDir)
      .filter((name) => name.endsWith(".rs") && name !== "mod.rs")
      .map((name) => name.replace(/\.rs$/, ""))
      .sort();

    // 源文件集合本身也要与前端清单一致：多一个源没接前端、少一个源没接后端，都是缺陷。
    expect(ids, "cli_registry/sources 的源文件集合与前端 CLI_IDS 不同").toEqual([...CLI_IDS].sort());

    // 显式列出的「纯前端位」必须真的存在，且不在可派生表里 —— 防止排除项本身写错而静默漏检。
    for (const bit of FRONTEND_ONLY_BITS) {
      expect(bit in CLI_DEFINITIONS.claude, `${bit} 不在 CliDefinition 上`).toBe(true);
      expect(bit in CAPABILITY_METHODS, `${bit} 不应同时有 Rust 判据`).toBe(false);
    }

    for (const id of ids) {
      const source = readFileSync(join(sourcesDir, `${id}.rs`), "utf8");
      const def = CLI_DEFINITIONS[id as CliId];
      expect(def, `CLI_DEFINITIONS 缺少 ${id}`).toBeDefined();
      for (const [bit, method] of Object.entries(CAPABILITY_METHODS)) {
        const rust = rustCapability(source, method);
        expect(rust, `${id}.rs 的 ${method}() 抽不到 Some/None（或 true/false）`).toBeDefined();
        expect(def[bit as keyof typeof def], `${id} 的 ${bit} 与 Rust ${method}() 不一致`).toBe(rust);
      }
    }
  });
});
