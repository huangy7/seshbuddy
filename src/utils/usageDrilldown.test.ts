import { describe, it, expect } from "vitest";
import {
  calculateModelBreakdown,
  calculateProjectBreakdown,
  getProjectName,
  getRecordCost,
  getModelPricing,
  type ModelBreakdownItem,
  type ProjectBreakdownItem,
} from "./usageDrilldown";
import type { UsageRecord } from "../types/session";
import { usePricingCatalog } from "../composables/usePricingCatalog";

const sampleRecords: UsageRecord[] = [
  {
    date: "2026-08-28",
    model: "claude-3-5-sonnet",
    input_tokens: 1000,
    output_tokens: 500,
    cache_creation_tokens: 100,
    cache_read_tokens: 200,
    duration_ms: 1200,
    project: "/Users/dev/workspace/SeshBuddy",
  },
  {
    date: "2026-08-28",
    model: "claude-3-5-sonnet",
    input_tokens: 500,
    output_tokens: 250,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    duration_ms: 800,
    project: "/Users/dev/workspace/OtherApp",
  },
  {
    date: "2026-08-28",
    model: "gemini-2.5-flash",
    input_tokens: 2000,
    output_tokens: 1000,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    duration_ms: 500,
    project: "/Users/dev/workspace/SeshBuddy",
  },
];

describe("usageDrilldown 双向穿透聚合计算", () => {
  describe("getProjectName 路径解析", () => {
    it("正确解析 Unix 风格路径", () => {
      expect(getProjectName("/Users/user/codes/SeshBuddy")).toBe("SeshBuddy");
      expect(getProjectName("/Users/user/codes/SeshBuddy/")).toBe("SeshBuddy");
    });

    it("正确解析 Windows 风格路径与反斜杠", () => {
      expect(getProjectName("C:\\Users\\huangy\\codes\\SeshBuddy")).toBe("SeshBuddy");
      expect(getProjectName("C:\\Users\\huangy\\codes\\SeshBuddy\\")).toBe("SeshBuddy");
      expect(getProjectName("D:/workspace/project\\")).toBe("project");
    });

    it("处理 default 与空路径", () => {
      expect(getProjectName("default")).toBe("Global default");
      expect(getProjectName("")).toBe("Global default");
    });
  });

  describe("getRecordCost 与 getModelPricing 计费计算", () => {
    it("精确计算 claude-3-5-sonnet 费用（含缓存写入与读取）", () => {
      const record: UsageRecord = {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 1_000_000, // 3 USD
        output_tokens: 1_000_000, // 15 USD
        cache_creation_tokens: 1_000_000, // 3.75 USD
        cache_read_tokens: 1_000_000, // 0.3 USD
        duration_ms: 100,
        project: "/proj",
      };
      // total = 3 + 15 + 3.75 + 0.3 = 22.05 USD
      expect(getRecordCost(record)).toBeCloseTo(22.05, 4);
    });

    it("精确计算 deepseek-chat 与 gpt-4o 等动态目录模型费用", () => {
      const deepseekRecord: UsageRecord = {
        date: "2026-08-28",
        model: "deepseek-chat",
        input_tokens: 1_000_000, // 0.27 USD
        output_tokens: 1_000_000, // 1.10 USD
        cache_creation_tokens: 1_000_000, // 0.27 USD
        cache_read_tokens: 1_000_000, // 0.07 USD
        duration_ms: 200,
        project: "/proj",
      };
      // total = 0.27 + 1.10 + 0.27 + 0.07 = 1.71 USD
      expect(getRecordCost(deepseekRecord)).toBeCloseTo(1.71, 4);

      const gpt4oRecord: UsageRecord = {
        date: "2026-08-28",
        model: "openai/gpt-4o",
        input_tokens: 1_000_000, // 2.50 USD
        output_tokens: 1_000_000, // 10.00 USD
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 300,
        project: "/proj",
      };
      expect(getRecordCost(gpt4oRecord)).toBeCloseTo(12.5, 4);
    });

    it("正确匹配未知模型并回退默认定价，不崩溃", () => {
      const record: UsageRecord = {
        date: "2026-08-28",
        model: "unknown-custom-model",
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 100,
        project: "/proj",
      };
      const pricing = getModelPricing("unknown-custom-model");
      expect(pricing).toBeDefined();
      expect(getRecordCost(record)).toBeGreaterThan(0);
      // 默认价格 3 + 15 = 18 USD
      expect(getRecordCost(record)).toBeCloseTo(18, 4);
    });

    it("支持动态修改价格目录后联动计算", () => {
      const { catalog } = usePricingCatalog();
      const original = { ...catalog.value };
      try {
        catalog.value["custom/super-model"] = {
          input_cost_per_token: 10 / 1_000_000,
          output_cost_per_token: 50 / 1_000_000,
          cache_creation_input_token_cost: 12.5 / 1_000_000,
          cache_read_input_token_cost: 1.0 / 1_000_000,
        };

        const record: UsageRecord = {
          date: "2026-08-28",
          model: "custom/super-model",
          input_tokens: 1_000_000,
          output_tokens: 1_000_000,
          cache_creation_tokens: 1_000_000,
          cache_read_tokens: 1_000_000,
          duration_ms: 100,
          project: "/proj",
        };

        // 10 + 50 + 12.5 + 1.0 = 73.5 USD
        expect(getRecordCost(record)).toBeCloseTo(73.5, 4);
      } finally {
        catalog.value = original;
      }
    });

    it("处理零 Token 消耗记录", () => {
      const record: UsageRecord = {
        date: "2026-08-28",
        model: "claude-3-5-sonnet",
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms: 0,
        project: "/proj",
      };
      expect(getRecordCost(record)).toBe(0);
    });
  });

  describe("calculateModelBreakdown 模型 ➔ 项目聚合", () => {
    it("聚合模型维度分布，并按 Token 降序排列每个模型下的项目", () => {
      const models = calculateModelBreakdown(sampleRecords);
      expect(models).toHaveLength(2);

      // 第一名是 gemini-2.5-flash (3000 tokens) 还是 sonnet (1800 + 750 = 2550 tokens)?
      // gemini: 2000 + 1000 = 3000 tokens
      // sonnet: (1000 + 500 + 100 + 200) + (500 + 250) = 1800 + 750 = 2550 tokens
      expect(models[0].model).toBe("gemini-2.5-flash");
      expect(models[0].tokens).toBe(3000);
      expect(models[0].projects).toHaveLength(1);
      expect(models[0].projects[0].projectName).toBe("SeshBuddy");
      expect(models[0].projects[0].percentage).toBe(100);

      const sonnet = models.find((m) => m.model === "claude-3-5-sonnet")!;
      expect(sonnet).toBeDefined();
      expect(sonnet.tokens).toBe(2550);
      expect(sonnet.projects).toHaveLength(2);
      expect(sonnet.projects[0].project).toBe("/Users/dev/workspace/SeshBuddy");
      expect(sonnet.projects[0].projectName).toBe("SeshBuddy");
      expect(sonnet.projects[0].tokens).toBe(1800);
      expect(sonnet.projects[0].percentage).toBeCloseTo((1800 / 2550) * 100, 2);

      expect(sonnet.projects[1].project).toBe("/Users/dev/workspace/OtherApp");
      expect(sonnet.projects[1].projectName).toBe("OtherApp");
      expect(sonnet.projects[1].tokens).toBe(750);
      expect(sonnet.projects[1].percentage).toBeCloseTo((750 / 2550) * 100, 2);
    });

    it("处理多条同一模型与同一项目的记录合并", () => {
      const records: UsageRecord[] = [
        {
          date: "2026-08-28",
          model: "claude-3-5-sonnet",
          input_tokens: 100,
          output_tokens: 100,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          duration_ms: null,
          project: "/app",
        },
        {
          date: "2026-08-28",
          model: "claude-3-5-sonnet",
          input_tokens: 200,
          output_tokens: 200,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          duration_ms: null,
          project: "/app",
        },
      ];
      const res = calculateModelBreakdown(records);
      expect(res).toHaveLength(1);
      expect(res[0].tokens).toBe(600);
      expect(res[0].projects).toHaveLength(1);
      expect(res[0].projects[0].tokens).toBe(600);
      expect(res[0].projects[0].percentage).toBe(100);
    });

    it("空记录数组返回空数组", () => {
      expect(calculateModelBreakdown([])).toEqual([]);
    });

    it("处理零 Token 记录，百分比不出现 NaN", () => {
      const records: UsageRecord[] = [
        {
          date: "2026-08-28",
          model: "claude-3-5-sonnet",
          input_tokens: 0,
          output_tokens: 0,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          duration_ms: 0,
          project: "/app",
        },
      ];
      const res = calculateModelBreakdown(records);
      expect(res).toHaveLength(1);
      expect(res[0].tokens).toBe(0);
      expect(res[0].projects[0].percentage).toBe(0);
      expect(Number.isNaN(res[0].projects[0].percentage)).toBe(false);
    });
  });

  describe("calculateProjectBreakdown 项目 ➔ 模型聚合", () => {
    it("聚合项目维度分布，并按费用降序排列项目与项目下的模型", () => {
      const projects = calculateProjectBreakdown(sampleRecords);
      expect(projects).toHaveLength(2);

      const seshbuddy = projects.find((p) => p.projectName === "SeshBuddy")!;
      expect(seshbuddy).toBeDefined();
      expect(seshbuddy.project).toBe("/Users/dev/workspace/SeshBuddy");
      expect(seshbuddy.tokens).toBe(1800 + 3000); // 4800
      expect(seshbuddy.models).toHaveLength(2);
      
      // 验证模型构成百分比
      const sonnetModel = seshbuddy.models.find((m) => m.model === "claude-3-5-sonnet")!;
      expect(sonnetModel.tokens).toBe(1800);
      expect(sonnetModel.percentage).toBeCloseTo((1800 / 4800) * 100, 2);

      const flashModel = seshbuddy.models.find((m) => m.model === "gemini-2.5-flash")!;
      expect(flashModel.tokens).toBe(3000);
      expect(flashModel.percentage).toBeCloseTo((3000 / 4800) * 100, 2);
    });

    it("空记录数组返回空数组", () => {
      expect(calculateProjectBreakdown([])).toEqual([]);
    });

    it("处理零 Token 记录，百分比不出现 NaN", () => {
      const records: UsageRecord[] = [
        {
          date: "2026-08-28",
          model: "claude-3-5-sonnet",
          input_tokens: 0,
          output_tokens: 0,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          duration_ms: 0,
          project: "/app",
        },
      ];
      const res = calculateProjectBreakdown(records);
      expect(res).toHaveLength(1);
      expect(res[0].tokens).toBe(0);
      expect(res[0].models[0].percentage).toBe(0);
      expect(Number.isNaN(res[0].models[0].percentage)).toBe(false);
    });
  });
});
