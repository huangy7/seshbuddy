import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";

// 不写成 `new URL("..", import.meta.url)`：那个字面形状是 Vite 的资源引用语法，会被改写成
// 非 file: 的模块 URL，fileURLToPath 直接抛「The URL must be of scheme file」。
// 经一个变量转手可避开该改写，实测取到的是本文件真实路径。
const TEST_FILE_URL = import.meta.url;
/** 本测试文件所在的 `src/` 目录。 */
const SRC_DIR = fileURLToPath(new URL("..", TEST_FILE_URL));

/** 身份值在本仓代码里可能写出的字面形态。三种引号都算：只认双引号的话，换种引号抄一份就绕过去了。 */
const LITERAL_FORMS = ['"Codex Official"', "'Codex Official'", "`Codex Official`"];

/** 文本里这个字面量出现了几次。 */
function countOfficialNameLiterals(source: string): number {
  return LITERAL_FORMS.reduce((n, form) => n + source.split(form).length - 1, 0);
}

/** `src/` 下的源码文件（绝对路径）。排除 `*.test.ts`，与闸门规则 1 的扫描口径一致。 */
function sourceFiles(): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const full = join(dir, name);
      if (statSync(full).isDirectory()) walk(full);
      else if (/\.(ts|vue)$/.test(full) && !full.endsWith(".test.ts")) out.push(full);
    }
  };
  walk(SRC_DIR);
  return out;
}

describe("OFFICIAL_PROFILE_NAME", () => {
  it("isOfficialProfile 只认官方配置名", async () => {
    const { OFFICIAL_PROFILE_NAME, isOfficialProfile } = await import("./officialProfile");
    expect(isOfficialProfile(OFFICIAL_PROFILE_NAME)).toBe(true);
    // 近失形态一律为假：判据是 `name === OFFICIAL_PROFILE_NAME`，不是前缀、不是忽略大小写的匹配。
    // 放宽成后两者会让 `Codex Official 2` 这类用户自建配置也进官方守卫，不可删不可改名。
    const others = [
      "Codex",
      "codex official",
      "Codex Official ",
      " Codex Official",
      "CodexOfficial",
      "Claude Official",
      "",
    ];
    for (const other of others) {
      expect(isOfficialProfile(other), `${JSON.stringify(other)} 不该被判成官方配置`).toBe(false);
    }
  });

  /**
   * 这条缺陷的失效模式是「有人又抄了一份」：两份字面量独立漂移时，官方配置的守卫
   * （不可删 / 不可改名 / 不可复制 / 不可编辑）只在一侧生效，而两侧各自看都是对的。
   * 唯一能机械挡住它的判据就是这个计数。
   */
  it("身份字面量只有共享模块一处（抄第三份就会红）", () => {
    // 阳性对照：判据在确实含该字面量的文本上报得出，证明下面的结果不是「恒返回空」。
    expect(countOfficialNameLiterals('const OFFICIAL_PROFILE_NAME = "Codex Official";')).toBe(1);
    expect(countOfficialNameLiterals('const OTHER = "Codex Officia";')).toBe(0);

    const files = sourceFiles();
    // 阳性对照：遍历确实读到了嵌套目录下的源码——否则「只有一个持有者」与「一个文件都没读」无法区分。
    expect(files).toContain(join(SRC_DIR, "components/ApiProfileManager.vue"));
    expect(files).toContain(join(SRC_DIR, "components/api-profile/ApiProfileList.vue"));

    const holders = files.filter((full) => countOfficialNameLiterals(readFileSync(full, "utf8")) > 0);
    expect(holders).toEqual([join(SRC_DIR, "utils/officialProfile.ts")]);
  });
});
