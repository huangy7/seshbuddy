import { t } from "../i18n";
import type { UsageRecord } from "../types/session";
import { getModelPricing } from "../composables/usePricingCatalog";

export { getModelPricing } from "../composables/usePricingCatalog";

export interface ModelProjectItem {
  project: string;
  projectName: string;
  tokens: number;
  cost: number;
  percentage: number;
}

export interface ModelBreakdownItem {
  model: string;
  tokens: number;
  cost: number;
  projects: ModelProjectItem[];
}

export interface ProjectModelItem {
  model: string;
  tokens: number;
  cost: number;
  percentage: number;
}

export interface ProjectBreakdownItem {
  project: string;
  projectName: string;
  tokens: number;
  cost: number;
  models: ProjectModelItem[];
}

export function getRecordCost(record: UsageRecord): number {
  const pricing = getModelPricing(record.model);
  const inputCost = (record.input_tokens || 0) * (pricing.input_cost_per_token ?? 0.000003);
  const outputCost = (record.output_tokens || 0) * (pricing.output_cost_per_token ?? 0.000015);
  const cacheWriteCost = (record.cache_creation_tokens || 0) * (pricing.cache_creation_input_token_cost ?? 0.00000375);
  const cacheReadCost = (record.cache_read_tokens || 0) * (pricing.cache_read_input_token_cost ?? 0.0000003);
  return inputCost + outputCost + cacheWriteCost + cacheReadCost;
}

export function getProjectName(path: string): string {
  if (!path || path === "default") return t("session.usageDashboard.defaultProject");
  const trimmed = path.replace(/[\\/]+$/, "");
  if (!trimmed) return t("session.usageDashboard.defaultProject");
  const parts = trimmed.split(/[\\/]/);
  const name = parts[parts.length - 1];
  return name || t("session.usageDashboard.defaultProject");
}

export function calculateModelBreakdown(records: UsageRecord[]): ModelBreakdownItem[] {
  const map = new Map<
    string,
    {
      tokens: number;
      cost: number;
      projectMap: Map<string, { tokens: number; cost: number }>;
    }
  >();

  for (const r of records) {
    const model = r.model || "unknown";
    const totalTokens =
      (r.input_tokens || 0) +
      (r.output_tokens || 0) +
      (r.cache_creation_tokens || 0) +
      (r.cache_read_tokens || 0);
    const cost = getRecordCost(r);
    const project = r.project || "default";

    let mEntry = map.get(model);
    if (!mEntry) {
      mEntry = { tokens: 0, cost: 0, projectMap: new Map() };
      map.set(model, mEntry);
    }
    mEntry.tokens += totalTokens;
    mEntry.cost += cost;

    const pEntry = mEntry.projectMap.get(project) || { tokens: 0, cost: 0 };
    pEntry.tokens += totalTokens;
    pEntry.cost += cost;
    mEntry.projectMap.set(project, pEntry);
  }

  const result: ModelBreakdownItem[] = [];
  for (const [model, mData] of map.entries()) {
    const projects: ModelProjectItem[] = [];
    for (const [proj, pData] of mData.projectMap.entries()) {
      projects.push({
        project: proj,
        projectName: getProjectName(proj),
        tokens: pData.tokens,
        cost: pData.cost,
        percentage: mData.tokens > 0 ? (pData.tokens / mData.tokens) * 100 : 0,
      });
    }
    projects.sort((a, b) => b.tokens - a.tokens || b.cost - a.cost);
    result.push({
      model,
      tokens: mData.tokens,
      cost: mData.cost,
      projects,
    });
  }

  return result.sort((a, b) => b.tokens - a.tokens || b.cost - a.cost);
}

export function calculateProjectBreakdown(records: UsageRecord[]): ProjectBreakdownItem[] {
  const map = new Map<
    string,
    {
      tokens: number;
      cost: number;
      modelMap: Map<string, { tokens: number; cost: number }>;
    }
  >();

  for (const r of records) {
    const project = r.project || "default";
    const totalTokens =
      (r.input_tokens || 0) +
      (r.output_tokens || 0) +
      (r.cache_creation_tokens || 0) +
      (r.cache_read_tokens || 0);
    const cost = getRecordCost(r);
    const model = r.model || "unknown";

    let pEntry = map.get(project);
    if (!pEntry) {
      pEntry = { tokens: 0, cost: 0, modelMap: new Map() };
      map.set(project, pEntry);
    }
    pEntry.tokens += totalTokens;
    pEntry.cost += cost;

    const mEntry = pEntry.modelMap.get(model) || { tokens: 0, cost: 0 };
    mEntry.tokens += totalTokens;
    mEntry.cost += cost;
    pEntry.modelMap.set(model, mEntry);
  }

  const result: ProjectBreakdownItem[] = [];
  for (const [proj, pData] of map.entries()) {
    const models: ProjectModelItem[] = [];
    for (const [model, mData] of pData.modelMap.entries()) {
      models.push({
        model,
        tokens: mData.tokens,
        cost: mData.cost,
        percentage: pData.tokens > 0 ? (mData.tokens / pData.tokens) * 100 : 0,
      });
    }
    models.sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
    result.push({
      project: proj,
      projectName: getProjectName(proj),
      tokens: pData.tokens,
      cost: pData.cost,
      models,
    });
  }

  return result.sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
}
