import { describe, expect, it } from "vitest";
import {
  getNestedValue,
  hasNestedKey,
  pruneEmptyObjects,
  stringifyConfigValue,
} from "./configValue";

describe("stringifyConfigValue", () => {
  it("字符串原样返回，数字与布尔转成字面量", () => {
    expect(stringifyConfigValue("sk-abc")).toBe("sk-abc");
    expect(stringifyConfigValue(42)).toBe("42");
    expect(stringifyConfigValue(0)).toBe("0");
    expect(stringifyConfigValue(false)).toBe("false");
  });

  it("对象与数组返回空串，绝不产生 [object Object]", () => {
    expect(stringifyConfigValue({ a: 1 })).toBe("");
    expect(stringifyConfigValue([1, 2])).toBe("");
    expect(stringifyConfigValue(null)).toBe("");
    expect(stringifyConfigValue(undefined)).toBe("");
  });
});

describe("getNestedValue", () => {
  it("按点分路径取到深层值", () => {
    const content = { env: { ANTHROPIC_BASE_URL: "https://x" } };
    expect(getNestedValue(content, "env.ANTHROPIC_BASE_URL")).toBe("https://x");
  });

  it("中间层缺失或不是对象时返回空串而非抛错", () => {
    expect(getNestedValue({}, "env.ANTHROPIC_BASE_URL")).toBe("");
    expect(getNestedValue({ env: "字符串" }, "env.key")).toBe("");
    expect(getNestedValue({ env: null }, "env.key")).toBe("");
  });

  it("路径末端是对象时按 stringifyConfigValue 规则返回空串", () => {
    expect(getNestedValue({ env: { nested: { deep: 1 } } }, "env.nested")).toBe("");
  });
});

describe("hasNestedKey", () => {
  it("叶子键存在即为 true，包括值为假值的键", () => {
    expect(hasNestedKey({ env: { K: "v" } }, ["env", "K"])).toBe(true);
    expect(hasNestedKey({ env: { K: false } }, ["env", "K"])).toBe(true);
    expect(hasNestedKey({ env: { K: 0 } }, ["env", "K"])).toBe(true);
    expect(hasNestedKey({ env: { K: null } }, ["env", "K"])).toBe(true);
  });

  it("键不存在、或中间层缺失时为 false", () => {
    expect(hasNestedKey({ env: {} }, ["env", "K"])).toBe(false);
    expect(hasNestedKey({}, ["env", "K"])).toBe(false);
    expect(hasNestedKey({ env: "字符串" }, ["env", "K"])).toBe(false);
  });

  it("数组不作为对象处理，其下标不算配置键", () => {
    expect(hasNestedKey({ env: [] }, ["env", "K"])).toBe(false);
    expect(hasNestedKey({ list: [{ K: 1 }] }, ["list", "K"])).toBe(false);
  });

  it("只认自有属性，不认原型链上的键", () => {
    expect(hasNestedKey({ env: {} }, ["env", "toString"])).toBe(false);
  });
});

describe("pruneEmptyObjects", () => {
  it("递归删除空对象，并从叶子向根连锁清理", () => {
    const value: Record<string, any> = { env: { nested: {} }, keep: 1 };
    pruneEmptyObjects(value);
    expect(value).toEqual({ keep: 1 });

    const deep: Record<string, any> = { a: { b: { c: {} } } };
    pruneEmptyObjects(deep);
    expect(deep).toEqual({});
  });

  it("非空对象与普通值不受影响", () => {
    const value: Record<string, any> = { env: { K: "v" }, n: 0, s: "", b: false, nul: null };
    pruneEmptyObjects(value);
    expect(value).toEqual({ env: { K: "v" }, n: 0, s: "", b: false, nul: null });
  });

  it("空数组不参与清理（空数组是有意义的配置值）", () => {
    const value: Record<string, any> = { list: [] };
    pruneEmptyObjects(value);
    expect(value).toEqual({ list: [] });
  });
});
