import { describe, expect, it, vi } from "vitest";

const invoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { renameSession } = await import("./renameSession");

describe("renameSession", () => {
  it("keeps CLI identity when same-path Claude and Codex conversations are renamed", async () => {
    await renameSession({ cliId: "claude", filePath: "/shared/session.jsonl" }, "Claude 对话");
    await renameSession({ cliId: "codex", filePath: "/shared/session.jsonl" }, "Codex 对话");

    expect(invoke).toHaveBeenNthCalledWith(1, "rename_session", {
      cliId: "claude",
      filePath: "/shared/session.jsonl",
      newName: "Claude 对话",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "rename_session", {
      cliId: "codex",
      filePath: "/shared/session.jsonl",
      newName: "Codex 对话",
    });
  });
});
