import { describe, expect, it } from "vitest";
import {
  buildVerbatimRegex,
  findOwnerFiles,
  splitFragments,
  stripComments,
  sweep,
} from "./check-zh-verbatim.mjs";

describe("splitFragments", () => {
  it("把占位符骨架切开，只留长度 > 1 的静态片段", () => {
    expect(splitFragments("Thought for {duration}")).toEqual(["Thought for"]);
    expect(splitFragments("Items: {count}")).toEqual(["Items:"]);
  });

  it("没有占位符时整串就是唯一片段", () => {
    expect(splitFragments("Thinking…")).toEqual(["Thinking…"]);
  });
});

describe("buildVerbatimRegex", () => {
  // 本工具存在的理由：子串判据会把这些标识符片段当成原文命中。
  it.each([
    "exportSelected",
    "startEdit",
    "toggleExportDropdown",
    "isFit",
    "todayStart",
    "watchersStopped",
  ])("不命中标识符片段 %s", (identifier) => {
    expect(buildVerbatimRegex("Selected").test(identifier)).toBe(false);
    expect(buildVerbatimRegex("Edit").test(identifier)).toBe(false);
    expect(buildVerbatimRegex("Fit").test(identifier)).toBe(false);
    expect(buildVerbatimRegex("today").test(identifier)).toBe(false);
    expect(buildVerbatimRegex("Stopped").test(identifier)).toBe(false);
  });

  it("命中真正作为文案出现的写法", () => {
    expect(buildVerbatimRegex("Selected").test("<strong>Selected</strong>")).toBe(true);
    expect(buildVerbatimRegex("Selected").test('"Selected"')).toBe(true);
    expect(buildVerbatimRegex("Body").test(">Body<")).toBe(true);
  });

  it("模板字符串原文按静态片段匹配", () => {
    expect(
      buildVerbatimRegex("Thought for {duration}").test("`Thought for ${formatDuration(m)}`"),
    ).toBe(true);
  });

  it("无可用片段时返回 null，而不是空匹配", () => {
    expect(buildVerbatimRegex("{a}")).toBeNull();
  });
});

describe("stripComments", () => {
  it("去掉 Vue 模板注释与 JS 注释——注释里的英文不是原文", () => {
    expect(stripComments("<!-- API Key -->")).not.toContain("API Key");
    expect(stripComments("// ─── Changes ───")).not.toContain("Changes");
  });
});

describe("findOwnerFiles", () => {
  it("按 t(\"<键>\") 的引用点定位所属文件", () => {
    const sources = {
      "a.vue": 'const x = t("chat.pill.selected");',
      "b.vue": "const y = t('chat.pill.selected');",
      "c.vue": 'const z = t("chat.pill.other");',
    };
    expect(findOwnerFiles("chat.pill.selected", sources).sort()).toEqual(["a.vue", "b.vue"]);
  });

  it("测试文件不算所属文件——断言 en 取值是循环论证", () => {
    const sources = {
      "src/components/common/ElegantSelect.vue": 't("common.select.placeholder")',
      "src/components/common/ElegantSelect.test.ts": 't("common.select.placeholder")',
    };
    expect(findOwnerFiles("common.select.placeholder", sources)).toEqual([
      "src/components/common/ElegantSelect.vue",
    ]);
  });
});

describe("sweep", () => {
  // 全树扫描的失效形状：别的组件里独立存在同一个英文词，与该键的原文无关。
  // 若不做所属文件限定，这里会把正确的中文 zh 判成 FAIL。
  it("只看所属文件，不被别的文件里的同名词击中", () => {
    const current = {
      "src/locales/en/chat.json": JSON.stringify({ pill: { cumulative: "Cumulative" } }),
      "src/locales/zh/chat.json": JSON.stringify({ pill: { cumulative: "累计" } }),
      "src/components/chat/Pill.vue": 't("chat.pill.cumulative")',
      "src/components/Other.vue": "<span>Cumulative</span>",
    };
    const base = {
      "src/components/chat/Pill.vue": "累计 <strong>{{ n }}</strong>",
      "src/components/Other.vue": "<span>Cumulative</span>",
    };
    const r = sweep({
      locales: ["chat"],
      readCurrent: (p) => current[p],
      readBase: (p) => base[p] ?? null,
      sources: current,
    });
    expect(r.hits).toEqual([]);
    expect(r.failures).toEqual([]);
  });

  // 键名自身含有 en 取值的静态片段，不抹掉就是自我命中。
  it("t('app.batch.selected') 的键名不被 en 取值 {count} selected 命中", () => {
    const current = {
      "src/locales/en/app.json": JSON.stringify({ batch: { selected: "{count} selected" } }),
      "src/locales/zh/app.json": JSON.stringify({ batch: { selected: "已选 {count} 个对话" } }),
      "src/App.vue": 't("app.batch.selected", { count: n })',
    };
    const r = sweep({
      locales: ["app"],
      readCurrent: (p) => current[p],
      readBase: (p) => current[p],
      sources: current,
    });
    expect(r.hits).toEqual([]);
  });

  it("所属文件的 base 里逐字出现、而 zh 被翻译时判 FAIL", () => {
    const current = {
      "src/locales/en/chat.json": JSON.stringify({ pill: { selected: "Selected" } }),
      "src/locales/zh/chat.json": JSON.stringify({ pill: { selected: "已选" } }),
      "src/components/chat/Pill.vue": 't("chat.pill.selected")',
    };
    const base = { "src/components/chat/Pill.vue": "<strong>Selected</strong>{{ n }}" };
    const r = sweep({
      locales: ["chat"],
      readCurrent: (p) => current[p],
      readBase: (p) => base[p] ?? null,
      sources: current,
    });
    expect(r.failures.map((f) => f.key)).toEqual(["chat.pill.selected"]);
  });

  // 这一条同时是整组用例的阳性对照：它断言 hits 非空，
  // 于是上面那些断言 hits 为空的用例才证明得了「确实没有命中」而不是「管道根本没通」。
  it("zh 逐字等于 en 时通过", () => {
    const current = {
      "src/locales/en/chat.json": JSON.stringify({ pill: { selected: "Selected" } }),
      "src/locales/zh/chat.json": JSON.stringify({ pill: { selected: "Selected" } }),
      "src/components/chat/Pill.vue": 't("chat.pill.selected")',
    };
    const base = { "src/components/chat/Pill.vue": "<strong>Selected</strong>{{ n }}" };
    const r = sweep({
      locales: ["chat"],
      readCurrent: (p) => current[p],
      readBase: (p) => base[p] ?? null,
      sources: current,
    });
    expect(r.failures).toEqual([]);
    expect(r.hits.map((h) => h.key)).toEqual(["chat.pill.selected"]);
  });

  // 静默跳过是「覆盖率看起来 100%」的成因——跳过必须被计数并报出来。
  it("无所属文件的键被计入 noOwner，而不是静默跳过", () => {
    const current = {
      "src/locales/en/chat.json": JSON.stringify({ a: { used: "Used" }, b: { dead: "Dead" } }),
      "src/locales/zh/chat.json": JSON.stringify({ a: { used: "已用" }, b: { dead: "已废弃" } }),
      "src/components/chat/Pill.vue": 't("chat.a.used")',
    };
    const r = sweep({
      locales: ["chat"],
      readCurrent: (p) => current[p],
      readBase: () => null,
      sources: current,
    });
    expect(r.scanned).toBe(1);
    expect(r.noOwner).toBe(1);
  });

  it("base 里不存在的文件（本批新增）不构成命中", () => {
    const current = {
      "src/locales/en/chat.json": JSON.stringify({ pill: { selected: "Selected" } }),
      "src/locales/zh/chat.json": JSON.stringify({ pill: { selected: "已选" } }),
      "src/components/chat/Pill.vue": 't("chat.pill.selected")',
    };
    const r = sweep({
      locales: ["chat"],
      readCurrent: (p) => current[p],
      readBase: () => null,
      sources: current,
    });
    expect(r.hits).toEqual([]);
  });
});
