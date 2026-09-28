import { describe, expect, it, vi } from "vitest";
import { useWorkspaces } from "./useWorkspaces";
import { findBestProjectPath } from "../utils/projectPath";
import type { ProjectInfo } from "../types/session";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

function project(originalPath: string | null): ProjectInfo {
  return { encoded_dir: null, original_path: originalPath, sessions: [] };
}

describe("useWorkspaces.mergedProjectPaths", () => {
  it("解析不出项目路径的会话不贡献工作区路径", () => {
    const { mergedProjectPaths } = useWorkspaces();

    expect(mergedProjectPaths([project(null), project("/Users/me/projA")])).toEqual([
      "/Users/me/projA",
    ]);
  });

  /**
   * 阳性对照：证明上面那条「跳过」不是恒真。
   *
   * 空串一旦混进工作区列表就会**匹配任何绝对路径**（`isSubPath` 的 `startsWith("/")` 分支），
   * 于是「没有项目路径」的会话会被算进随便某个项目。判据是这条性质本身，不是调用方的写法。
   */
  it("空串会匹配任何绝对路径，故不能拿它顶替缺席", () => {
    const file = "/Users/me/projA/s1.jsonl";

    expect(findBestProjectPath(file, [""])).toBe("");
    expect(findBestProjectPath(file, [])).toBe(null);
    expect(findBestProjectPath(file, ["/Users/me/projA"])).toBe("/Users/me/projA");
  });
});
