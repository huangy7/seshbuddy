import { describe, expect, it, afterEach, beforeEach } from "vitest";
import { setLocale } from "../i18n";
import { formatCost, formatCostCompact, formatTokenCount, formatNumber, formatDate } from "./format";

describe("formatCost", () => {
  // formatCost 现在跟随 app locale（分隔符随语言变），所以这几条断言必须自己钉住 en，
  // 而不是依赖「本文件跑到现在还没人改过 locale」这种文件顺序上的巧合。
  beforeEach(() => setLocale("en"));

  it("formats with thousand separators and two decimals", () => {
    expect(formatCost(1234.56)).toBe("$1,234.56");
    expect(formatCost(673.66)).toBe("$673.66");
    expect(formatCost(0)).toBe("$0.00");
  });

  it("keeps the sign for negative values", () => {
    expect(formatCost(-12.5)).toBe("-$12.50");
  });

  it("treats non-finite input as zero", () => {
    expect(formatCost(NaN)).toBe("$0.00");
    expect(formatCost(Infinity)).toBe("$0.00");
  });
});

describe("formatCostCompact", () => {
  it("keeps two decimals below 100", () => {
    expect(formatCostCompact(0)).toBe("$0.00");
    expect(formatCostCompact(67.33)).toBe("$67.33");
    expect(formatCostCompact(99.99)).toBe("$99.99");
  });

  it("drops to one decimal from 100 up to 1000", () => {
    expect(formatCostCompact(100)).toBe("$100.0");
    expect(formatCostCompact(673.66)).toBe("$673.7");
    expect(formatCostCompact(999.94)).toBe("$999.9");
  });

  it("uses K suffix at thousands", () => {
    expect(formatCostCompact(999.95)).toBe("$1.00K");
    expect(formatCostCompact(1234.56)).toBe("$1.23K");
    expect(formatCostCompact(999994)).toBe("$999.99K");
  });

  it("uses M suffix at millions", () => {
    expect(formatCostCompact(999995)).toBe("$1.00M");
    expect(formatCostCompact(1_500_000)).toBe("$1.50M");
  });

  it("keeps the sign and handles non-finite input", () => {
    expect(formatCostCompact(-673.66)).toBe("-$673.7");
    expect(formatCostCompact(NaN)).toBe("$0.00");
  });
});

describe("formatTokenCount", () => {
  it("千以下原样显示", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(380)).toBe("380");
    expect(formatTokenCount(999)).toBe("999");
  });

  it("千级显示 k，一位小数并去尾零", () => {
    expect(formatTokenCount(1200)).toBe("1.2k");
    expect(formatTokenCount(5000)).toBe("5k");
    expect(formatTokenCount(12400)).toBe("12.4k");
  });

  it("百万级显示 M", () => {
    expect(formatTokenCount(2_500_000)).toBe("2.5M");
    expect(formatTokenCount(1_000_000)).toBe("1M");
  });

  it("异常输入兜底为 0", () => {
    expect(formatTokenCount(NaN)).toBe("0");
    expect(formatTokenCount(-5)).toBe("0");
  });
});

describe("formatNumber / formatDate 跟随 app locale", () => {
  afterEach(() => setLocale("en"));

  it("同一数值在 en 与 de 下产出不同分隔", () => {
    setLocale("en");
    const en = formatNumber(1234567.89);
    setLocale("de");
    const de = formatNumber(1234567.89);
    expect(en).not.toBe(de);                   // 断言「不同」而不是断言具体串
  });

  it("不依赖运行时默认 locale", () => {
    // 关键：无论宿主环境的默认 locale 是什么，en 下的产出必须一致
    setLocale("en");
    expect(formatNumber(1234567.89)).toBe(new Intl.NumberFormat("en").format(1234567.89));
  });

  it("formatDate 跟随 locale 而非宿主默认", () => {
    setLocale("de");
    const d = new Date(Date.UTC(2026, 0, 2));
    expect(formatDate(d)).toBe(new Intl.DateTimeFormat("de").format(d));
  });
});
