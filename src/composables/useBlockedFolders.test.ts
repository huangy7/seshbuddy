import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockedInvoke } = vi.hoisted(() => ({ mockedInvoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockedInvoke,
}));

let useBlockedFolders: typeof import("./useBlockedFolders").useBlockedFolders;

beforeEach(async () => {
  vi.resetModules();
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "read_blocked_folders") return [];
    return null;
  });
  ({ useBlockedFolders } = await import("./useBlockedFolders"));
});

describe("useBlockedFolders", () => {
  it("loadBlockedFolders 读取后端屏蔽文件夹列表", async () => {
    const stored = ["/workspace/secret", "/workspace/temp"];
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "read_blocked_folders") return stored;
      return null;
    });

    const { blockedFolders, loadBlockedFolders, isBlocked } = useBlockedFolders();
    expect(blockedFolders.value).toEqual([]);
    expect(isBlocked("/workspace/secret")).toBe(false);

    await loadBlockedFolders();
    expect(mockedInvoke).toHaveBeenCalledWith("read_blocked_folders");
    expect(blockedFolders.value).toEqual(stored);
    expect(isBlocked("/workspace/secret")).toBe(true);
    expect(isBlocked("/workspace/secret/subfolder/file.ts")).toBe(true);
    expect(isBlocked("/workspace/other")).toBe(false);
  });

  describe("前缀匹配规则 (isBlocked)", () => {
    it("精确匹配与子目录匹配生效，相似名称但非子目录不匹配", async () => {
      const { blockFolder, isBlocked } = useBlockedFolders();
      await blockFolder("/a/b");

      // 精确匹配
      expect(isBlocked("/a/b")).toBe(true);
      // Unix 子目录
      expect(isBlocked("/a/b/c")).toBe(true);
      expect(isBlocked("/a/b/sub/deep")).toBe(true);
      // Windows 反斜杠子目录
      expect(isBlocked("/a/b\\c")).toBe(true);

      // 非子目录（前缀字符串相同但无路径分隔符）
      expect(isBlocked("/a/bc")).toBe(false);
      expect(isBlocked("/a/b_other")).toBe(false);

      // 父目录不被匹配
      expect(isBlocked("/a")).toBe(false);
      expect(isBlocked("/")).toBe(false);
    });

    it("Windows 路径风格前缀匹配", async () => {
      const { blockFolder, isBlocked } = useBlockedFolders();
      await blockFolder("C:\\Projects\\Secret");

      expect(isBlocked("C:\\Projects\\Secret")).toBe(true);
      expect(isBlocked("C:\\Projects\\Secret\\app")).toBe(true);
      expect(isBlocked("C:\\Projects\\Secret/app")).toBe(true);
      expect(isBlocked("C:\\Projects\\Secret2")).toBe(false);
    });

    it("末尾斜杠不影响匹配", async () => {
      const { blockFolder, isBlocked } = useBlockedFolders();
      await blockFolder("/a/b/");

      expect(isBlocked("/a/b")).toBe(true);
      expect(isBlocked("/a/b/")).toBe(true);
      expect(isBlocked("/a/b/sub")).toBe(true);
    });
  });

  it("blockFolder 与 unblockFolder 交互与去重", async () => {
    const { blockedFolders, blockFolder, unblockFolder } = useBlockedFolders();

    await blockFolder("/path/one");
    await blockFolder("/path/two");
    // 重复添加不增加
    await blockFolder("/path/one");

    const writes = () =>
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "write_blocked_folders");

    expect(blockedFolders.value).toEqual(["/path/one", "/path/two"]);
    expect(writes().length).toBe(2);
    expect(writes()[0][1]).toEqual({ paths: ["/path/one"] });
    expect(writes()[1][1]).toEqual({ paths: ["/path/one", "/path/two"] });

    // 解除屏蔽
    await unblockFolder("/path/one");
    expect(blockedFolders.value).toEqual(["/path/two"]);
    expect(writes()[2][1]).toEqual({ paths: ["/path/two"] });
  });
});
