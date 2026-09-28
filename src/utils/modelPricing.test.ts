import { describe, expect, it } from "vitest";
import { buildPriceById, formatModelPrice, formatModelTooltip, parseBillingRules } from "./modelPricing";

describe("parseBillingRules", () => {
  it("extracts all five coefficients from a full formula", () => {
    const coeffs = parseBillingRules(
      'tier("base", p * 5 + c * 25 + cr * 0.5 + cc * 6.25 + cc1h * 10)',
    );
    expect(coeffs).toEqual({ p: 5, c: 25, cr: 0.5, cc: 6.25, cc1h: 10 });
  });

  it("extracts partial coefficients", () => {
    expect(parseBillingRules('tier("base", p * 2 + c * 8)')).toEqual({ p: 2, c: 8 });
  });

  it("does not confuse c with cr/cc/cc1h", () => {
    const coeffs = parseBillingRules('tier("base", cr * 1 + cc * 2 + cc1h * 3)');
    expect(coeffs.c).toBeUndefined();
    expect(coeffs.cr).toBe(1);
    expect(coeffs.cc).toBe(2);
    expect(coeffs.cc1h).toBe(3);
  });

  it("returns empty object for missing or unrecognized formulas", () => {
    expect(parseBillingRules(undefined)).toEqual({});
    expect(parseBillingRules("")).toEqual({});
    expect(parseBillingRules("free")).toEqual({});
  });

  it("takes the first tier's coefficients when multiple tiers exist", () => {
    const coeffs = parseBillingRules(
      'tier("base", p * 5 + c * 25) + tier("long", len > 200000, p * 10 + c * 50)',
    );
    expect(coeffs.p).toBe(5);
    expect(coeffs.c).toBe(25);
  });
});

describe("formatModelPrice", () => {
  it("formats in/out prices with $ prefix", () => {
    expect(formatModelPrice({ p: 5, c: 25 })).toBe("$5/$25");
    expect(formatModelPrice({ p: 0.5, c: 2.5 })).toBe("$0.5/$2.5");
  });

  it("uses — for a missing output price", () => {
    expect(formatModelPrice({ p: 5 })).toBe("$5/—");
  });

  it("returns null when there is no input price", () => {
    expect(formatModelPrice({})).toBeNull();
    expect(formatModelPrice({ c: 25 })).toBeNull();
  });
});

describe("formatModelTooltip", () => {
  const priceById = new Map([["kimi-k2.6", "$2/$8"]]);

  it("appends the price when available", () => {
    expect(formatModelTooltip("kimi-k2.6", priceById)).toBe("kimi-k2.6 · $2/$8 per M");
  });

  it("falls back to the bare id when no price or no map", () => {
    expect(formatModelTooltip("unknown", priceById)).toBe("unknown");
    expect(formatModelTooltip("kimi-k2.6", undefined)).toBe("kimi-k2.6");
  });
});

describe("buildPriceById", () => {
  it("maps id to formatted price, skipping entries without billing info", () => {
    const map = buildPriceById([
      { id: "kimi-k2.6", billing_rules: 'tier("base", p * 2 + c * 8)' },
      { id: "free-model", billing_rules: null },
      { id: "no-rules" },
    ]);
    expect(map.get("kimi-k2.6")).toBe("$2/$8");
    expect(map.has("free-model")).toBe(false);
    expect(map.has("no-rules")).toBe(false);
  });
});
