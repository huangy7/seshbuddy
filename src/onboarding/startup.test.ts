import { describe, expect, it } from "vitest";
import { completeFirstRun, loadStartupMode, runStartup } from "./startup";

describe("first-run startup", () => {
  it("maps a required setup to onboarding", async () => {
    expect(await loadStartupMode(async () => true)).toBe("onboarding");
    expect(await loadStartupMode(async () => false)).toBe("ready");
  });

  it("keeps the setup decision recoverable when the backend read fails", async () => {
    expect(await loadStartupMode(async () => { throw new Error("unavailable"); })).toBe("status-error");
  });

  it("detects CLI sources without starting session work during onboarding", async () => {
    const order: string[] = [];
    await runStartup("onboarding", {
      detectClis: async () => { order.push("detect"); },
      startSessions: async () => { order.push("scan"); },
    });
    expect(order).toEqual(["detect"]);
  });

  it("saves language and CLI choices before marking setup complete", async () => {
    const order: string[] = [];
    await completeFirstRun(
      { locale: "ja", cliIds: ["codex"] },
      {
        saveLocale: async () => { order.push("language"); return true; },
        saveFilter: () => { order.push("filter"); },
        markComplete: async () => { order.push("complete"); },
      },
    );
    expect(order).toEqual(["language", "filter", "complete"]);
  });

  it("does not mark setup complete when the language write fails", async () => {
    const order: string[] = [];
    await expect(completeFirstRun(
      { locale: "de", cliIds: ["gemini"] },
      {
        saveLocale: async () => false,
        saveFilter: () => { order.push("filter"); },
        markComplete: async () => { order.push("complete"); },
      },
    )).rejects.toThrow();
    expect(order).toEqual([]);
  });

  it("does not mark setup complete when saving the CLI filter fails", async () => {
    const order: string[] = [];
    await expect(completeFirstRun(
      { locale: "en", cliIds: ["claude"] },
      {
        saveLocale: async () => { order.push("language"); return true; },
        saveFilter: () => { throw new Error("storage unavailable"); },
        markComplete: async () => { order.push("complete"); },
      },
    )).rejects.toThrow("storage unavailable");
    expect(order).toEqual(["language"]);
  });
});
