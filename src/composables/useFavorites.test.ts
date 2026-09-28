import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FavoriteEntry } from "./useFavorites";

const { mockedInvoke } = vi.hoisted(() => ({ mockedInvoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockedInvoke,
}));

let useFavorites: typeof import("./useFavorites").useFavorites;

beforeEach(async () => {
  vi.resetModules();
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "read_favorite_entries") return [];
    return null;
  });
  ({ useFavorites } = await import("./useFavorites"));
});

describe("useFavorites", () => {
  it("toggle 后 isFavorite 为 true，再次 toggle 取消", async () => {
    const { isFavorite, toggleFavorite } = useFavorites();
    const identity = { cliId: "claude", filePath: "/a.jsonl" } as const;

    expect(isFavorite(identity)).toBe(false);
    await toggleFavorite(identity);
    expect(isFavorite(identity)).toBe(true);
    await toggleFavorite(identity);
    expect(isFavorite(identity)).toBe(false);
  });

  it("同路径不同 CLI 互不影响", async () => {
    const { isFavorite, toggleFavorite } = useFavorites();
    await toggleFavorite({ cliId: "claude", filePath: "/a.jsonl" });
    expect(isFavorite({ cliId: "claude", filePath: "/a.jsonl" })).toBe(true);
    expect(isFavorite({ cliId: "codex", filePath: "/a.jsonl" })).toBe(false);
  });

  it("toggle 调 write_favorite_entries 载荷为全量列表（snake_case 映射）", async () => {
    const { toggleFavorite } = useFavorites();
    await toggleFavorite({ cliId: "claude", filePath: "/a.jsonl" });
    await toggleFavorite({ cliId: "codex", filePath: "/b.jsonl" });

    const writes = () =>
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "write_favorite_entries");
    expect(writes()).toHaveLength(2);
    expect(writes()[0][1]).toEqual({
      entries: [{ cli_id: "claude", path: "/a.jsonl" }],
    });
    expect(writes()[1][1]).toEqual({
      entries: [
        { cli_id: "claude", path: "/a.jsonl" },
        { cli_id: "codex", path: "/b.jsonl" },
      ],
    });

    // 取消星标后写回剩余全量
    await toggleFavorite({ cliId: "claude", filePath: "/a.jsonl" });
    expect(writes().at(-1)?.[1]).toEqual({
      entries: [{ cli_id: "codex", path: "/b.jsonl" }],
    });
  });

  it("loadFavorites 读取后端条目并映射为可用状态", async () => {
    const stored: FavoriteEntry[] = [{ cli_id: "claude", path: "/x.jsonl" }];
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "read_favorite_entries") return stored;
      return null;
    });

    const { favorites, isFavorite, loadFavorites } = useFavorites();
    expect(isFavorite({ cliId: "claude", filePath: "/x.jsonl" })).toBe(false);

    await loadFavorites();
    expect(mockedInvoke).toHaveBeenCalledWith("read_favorite_entries");
    expect(favorites.value).toEqual(stored);
    expect(isFavorite({ cliId: "claude", filePath: "/x.jsonl" })).toBe(true);
    expect(isFavorite({ cliId: "codex", filePath: "/x.jsonl" })).toBe(false);
  });
});
