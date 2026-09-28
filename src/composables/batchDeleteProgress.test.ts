import { describe, it, expect } from "vitest";
import { batchDeleteProgressText } from "./batchDeleteProgress";

describe("batchDeleteProgressText", () => {
  it("无路径时只显示分数", () => {
    expect(batchDeleteProgressText({ done: 1, total: 10, currentPath: "" })).toBe("Deleting 1/10");
  });
  it("带路径时附上文件名", () => {
    expect(batchDeleteProgressText({ done: 3, total: 5, currentPath: "/a/b.jsonl" })).toBe("Deleting 3/5 · b.jsonl");
  });
  it("Windows 反斜杠路径也取文件名", () => {
    expect(batchDeleteProgressText({ done: 1, total: 2, currentPath: "C:\\a\\b.jsonl" })).toBe("Deleting 1/2 · b.jsonl");
  });
  it("完成态文案", () => {
    expect(batchDeleteProgressText({ done: 10, total: 10, currentPath: "" })).toBe("Deleting 10/10");
  });
});
