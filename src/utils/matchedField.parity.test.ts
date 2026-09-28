import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { matchedFieldLabel } from "./matchedField";
import { rustEnumVariants, tsUnionMembers } from "./enumParity";
import type { MatchedField } from "../types/session";
import { setLocale } from "../i18n";

// 不写成 `new URL("../../…", import.meta.url)`：那个字面形状是 Vite 的资源引用语法，
// 会被改写成非 file: 的模块 URL，fileURLToPath 直接抛「The URL must be of scheme file」。
// 经一个变量转手可避开该改写，实测取到的是本文件真实路径。
const TEST_FILE_URL = import.meta.url;
const REPO_ROOT = join(fileURLToPath(new URL("../../", TEST_FILE_URL)));
const BACKEND_SESSION_RS = join(REPO_ROOT, "src-tauri/src/session.rs");
const FRONTEND_SESSION_TS = join(REPO_ROOT, "src/types/session.ts");

/**
 * 提取器与折算口径见 `./enumParity`。本文件只钉 `MatchedField` 这一个镜像；
 * `ExportOutcome` 与 `TrafficSessionLabel` 同形的守卫在 `./enumParity.test.ts`。
 */
const rustMatchedFieldVariants = (source: string) => rustEnumVariants(source, "MatchedField");
const tsMatchedFieldVariants = (source: string) => tsUnionMembers(source, "MatchedField");

describe("MatchedField 的 Rust↔TS parity", () => {
  /**
   * 这条钉的是 `matchedField.ts` 的 `never` 哨兵**关不掉**的那半边：
   * Rust 加变体、TS 联合没跟上时，联合本身没变、`default` 不可达，`vue-tsc` 不响，
   * 而运行时的 `default` 会把字面量原样返回并粘上 snippet。只有比对两边的判据能关它。
   */
  it("Rust 枚举的变体集与 TS 联合的成员集合相同（口径：变体名按 snake_case 折算）", () => {
    // 阳性对照：判据在确实含枚举/联合的文本上报得出 —— 否则「两边一致」与
    // 「两边都抽到空集」无法区分，一个匹配不到东西的正则会让本测试恒绿。
    expect(rustMatchedFieldVariants("pub enum MatchedField {\n    Title,\n}")).toEqual(["title"]);
    expect(rustMatchedFieldVariants("没有枚举的文本")).toEqual([]);
    expect(tsMatchedFieldVariants('export type MatchedField = "title";')).toEqual(["title"]);
    expect(tsMatchedFieldVariants("没有联合的文本")).toEqual([]);

    const rust = rustMatchedFieldVariants(readFileSync(BACKEND_SESSION_RS, "utf8"));
    const ts = tsMatchedFieldVariants(readFileSync(FRONTEND_SESSION_TS, "utf8"));
    expect(rust, `${BACKEND_SESSION_RS} 里没抽到任何变体`).not.toEqual([]);
    expect(ts, `${FRONTEND_SESSION_TS} 里没抽到任何成员`).not.toEqual([]);
    expect(ts).toEqual(rust);
  });

  /**
   * 行为侧的同一件事：把 Rust 侧的**每一个**变体名喂进标签函数。
   * TS 联合若漏了这个变体，运行时就落进 `default` 并把字面量回显 —— 这条会红。
   * 这是「UI 把 `session_id` 粘在 snippet 上」那个失效模式的直接判据。
   */
  it("Rust 侧每个变体都渲染成标签或空串，没有变体把自己回显上屏", () => {
    setLocale("en");
    const rust = rustMatchedFieldVariants(readFileSync(BACKEND_SESSION_RS, "utf8"));
    expect(rust).not.toEqual([]);
    for (const variant of rust) {
      const label = matchedFieldLabel(variant as MatchedField);
      expect(label, `变体 ${variant} 被原样回显`).not.toBe(variant);
    }
  });
});
