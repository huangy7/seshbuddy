import { describe, expect, it } from "vitest";
import {
  adoptLoadedNames,
  buildDisplayById,
  fillAutoNames,
  parseCachedModelEntries,
  type ModelEntry,
} from "./modelDisplayName";

const entries: ModelEntry[] = [
  { id: "kimi-k2.6", display: "Kimi K2.6" },
  { id: "claude-sonnet-4-5", display: "Claude Sonnet 4.5" },
  { id: "no-display" },
];

const none = { haiku: false, sonnet: false, opus: false };

describe("buildDisplayById", () => {
  it("maps id to display, skipping entries without display", () => {
    const map = buildDisplayById(entries);
    expect(map.get("kimi-k2.6")).toBe("Kimi K2.6");
    expect(map.get("claude-sonnet-4-5")).toBe("Claude Sonnet 4.5");
    expect(map.has("no-display")).toBe(false);
  });

  it("ignores empty or whitespace-only display", () => {
    const map = buildDisplayById([{ id: "a", display: "  " }, { id: "b", display: "" }]);
    expect(map.has("a")).toBe(false);
    expect(map.has("b")).toBe(false);
  });
});

describe("adoptLoadedNames", () => {
  it("marks a loaded name that differs from the display as hand-typed", () => {
    expect(
      adoptLoadedNames(
        { haiku: "", sonnet: "我的主力模型", opus: "" },
        { haiku: "", sonnet: "Claude Sonnet 4.5", opus: "" },
        none,
      ),
    ).toEqual({ haiku: false, sonnet: true, opus: false });
  });

  it("leaves a loaded name equal to the display untyped, so old auto-fills migrate", () => {
    expect(
      adoptLoadedNames(
        { haiku: "", sonnet: "Claude Sonnet 4.5", opus: "" },
        { haiku: "", sonnet: "Claude Sonnet 4.5", opus: "" },
        none,
      ),
    ).toEqual(none);
  });

  it("leaves empty names untyped", () => {
    expect(
      adoptLoadedNames(
        { haiku: "", sonnet: "", opus: "" },
        { haiku: "Kimi K2.6", sonnet: "Claude Sonnet 4.5", opus: "" },
        none,
      ),
    ).toEqual(none);
  });

  it("never un-marks a role the user already typed in", () => {
    expect(
      adoptLoadedNames(
        { haiku: "", sonnet: "Claude Sonnet 4.5", opus: "" },
        { haiku: "", sonnet: "Claude Sonnet 4.5", opus: "" },
        { haiku: false, sonnet: true, opus: false },
      ),
    ).toEqual({ haiku: false, sonnet: true, opus: false });
  });

  it("ignores a whitespace-only loaded name", () => {
    expect(
      adoptLoadedNames(
        { haiku: "", sonnet: "   ", opus: "" },
        { haiku: "", sonnet: "Claude Sonnet 4.5", opus: "" },
        none,
      ),
    ).toEqual(none);
  });
});

describe("fillAutoNames", () => {
  it("fills every untyped role with its display", () => {
    expect(
      fillAutoNames(
        { haiku: "Kimi K2.6", sonnet: "Claude Sonnet 4.5", opus: "" },
        none,
      ),
    ).toEqual({ haiku: "Kimi K2.6", sonnet: "Claude Sonnet 4.5", opus: "" });
  });

  it("skips roles the user typed in", () => {
    expect(
      fillAutoNames(
        { haiku: "Kimi K2.6", sonnet: "Claude Sonnet 4.5", opus: "" },
        { haiku: false, sonnet: true, opus: false },
      ),
    ).toEqual({ haiku: "Kimi K2.6", opus: "" });
  });

  it("clears an untyped role whose model has no display", () => {
    expect(
      fillAutoNames({ haiku: "", sonnet: "Kimi K2.6", opus: "" }, none),
    ).toEqual({ haiku: "", sonnet: "Kimi K2.6", opus: "" });
  });
});

describe("parseCachedModelEntries", () => {
  it("parses the new {models:[{id,display}]} format", () => {
    const raw = JSON.stringify({
      base_url: "https://gw.example.com",
      models: [{ id: "kimi-k2.6", display: "Kimi K2.6" }],
      fetched_at: "2026-09-03T00:00:00Z",
    });
    expect(parseCachedModelEntries(raw)).toEqual([{ id: "kimi-k2.6", display: "Kimi K2.6" }]);
  });

  it("upgrades the legacy {model_ids: string[]} format", () => {
    const raw = JSON.stringify({
      base_url: "https://gw.example.com",
      model_ids: ["kimi-k2.6", "claude-sonnet-4-5"],
      fetched_at: "2026-09-03T00:00:00Z",
    });
    expect(parseCachedModelEntries(raw)).toEqual([
      { id: "kimi-k2.6" },
      { id: "claude-sonnet-4-5" },
    ]);
  });

  it("returns empty array for missing or malformed payloads", () => {
    expect(parseCachedModelEntries(null)).toEqual([]);
    expect(parseCachedModelEntries("not json")).toEqual([]);
    expect(parseCachedModelEntries("{}")).toEqual([]);
    expect(parseCachedModelEntries(JSON.stringify({ models: "nope" }))).toEqual([]);
  });
});
