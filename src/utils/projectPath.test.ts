import { describe, it, expect } from "vitest";
import { basename, isSubPath, findBestProjectPath } from "./projectPath";

describe("projectPath.ts", () => {
  it("extracts basename correctly for POSIX and Windows paths", () => {
    expect(basename("/Users/test/codes/SeshBuddy")).toBe("SeshBuddy");
    expect(basename("/Users/test/codes/SeshBuddy/")).toBe("SeshBuddy");
    expect(basename("C:\\Users\\test\\codes\\SeshBuddy")).toBe("SeshBuddy");
    expect(basename("C:\\Users\\test\\codes\\SeshBuddy\\")).toBe("SeshBuddy");
    expect(basename("SeshBuddy")).toBe("SeshBuddy");
    expect(basename("")).toBe("");
  });

  it("checks subpaths correctly", () => {
    expect(isSubPath("/Users/test/codes/SeshBuddy/src", "/Users/test/codes/SeshBuddy")).toBe(true);
    expect(isSubPath("C:\\Users\\test\\codes\\SeshBuddy\\src", "C:\\Users\\test\\codes\\SeshBuddy")).toBe(true);
    expect(isSubPath("/Users/test/other", "/Users/test/codes/SeshBuddy")).toBe(false);
  });

  it("finds best matching project path", () => {
    const list = ["/Users/test/codes", "/Users/test/codes/SeshBuddy"];
    expect(findBestProjectPath("/Users/test/codes/SeshBuddy/src/App.vue", list)).toBe("/Users/test/codes/SeshBuddy");
  });
});
