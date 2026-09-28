import { describe, expect, it } from "vitest";
import { has1m, set1m, strip1m } from "./modelSuffix";

describe("has1m", () => {
  it("returns false for empty or undefined", () => {
    expect(has1m(undefined)).toBe(false);
    expect(has1m("")).toBe(false);
  });

  it("detects the [1m] suffix case-insensitively, ignoring trailing spaces", () => {
    expect(has1m("opus[1m]")).toBe(true);
    expect(has1m("OPUS[1M]  ")).toBe(true);
    expect(has1m("opus")).toBe(false);
    expect(has1m("opus[1m]x")).toBe(false);
  });
});

describe("strip1m", () => {
  it("returns empty string for undefined", () => {
    expect(strip1m(undefined)).toBe("");
  });

  it("removes the [1m] suffix and trailing whitespace before it", () => {
    expect(strip1m("opus[1m]")).toBe("opus");
    expect(strip1m("opus [1m] ")).toBe("opus");
  });

  it("returns the input unchanged when no suffix", () => {
    expect(strip1m("opus")).toBe("opus");
    expect(strip1m(" opus ")).toBe(" opus ");
  });
});

describe("set1m", () => {
  it("appends or removes the suffix on the stripped base", () => {
    expect(set1m("opus", true)).toBe("opus[1m]");
    expect(set1m("opus[1m]", true)).toBe("opus[1m]");
    expect(set1m("opus[1m]", false)).toBe("opus");
  });

  it("returns empty string when the base is blank", () => {
    expect(set1m("", true)).toBe("");
    expect(set1m("   ", true)).toBe("");
  });
});
