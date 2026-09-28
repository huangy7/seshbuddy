import { t } from "../i18n";
import { formatNumber } from "./format";
import type { ChatMessage, SessionStats } from "../types/session";

export interface ToolDistributionItem {
  key: string;
  label: string;
  count: number;
  share: number; // 0..1
}

export interface TokenTimelinePoint {
  index: number;
  timestamp: number | null;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  total: number;
  cumulative: number;
}

export interface TokenTimelineBucket {
  key: string;
  startIndex: number;
  endIndex: number;
  startTime: number | null;
  endTime: number | null;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  total: number;
  cumulative: number;
}

export interface SessionAnalytics {
  roleCounts: {
    user: number;
    assistant: number;
    tool: number;
    system: number;
  };
  toolCalls: number;
  toolTypes: number;
  toolDistribution: ToolDistributionItem[];
  tokenPoints: TokenTimelinePoint[];
  tokenBuckets: TokenTimelineBucket[];
  tokenTotals: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    total: number;
  };
  firstTimestamp: number | null;
  lastTimestamp: number | null;
  peakTokenPoint: TokenTimelinePoint | null;
  averageTokensPerAssistantTurn: number | null;
  toolsPerUserTurn: number | null;
  cacheShare: number | null;
  estimatedCost: number;
}

const MAX_TOKEN_BUCKETS = 44;
const MODEL_PRICING_DEFAULT = { input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3 };

function buildTokenBuckets(points: TokenTimelinePoint[]): TokenTimelineBucket[] {
  if (points.length === 0) return [];
  const bucketSize = Math.max(1, Math.ceil(points.length / MAX_TOKEN_BUCKETS));
  const buckets: TokenTimelineBucket[] = [];

  for (let i = 0; i < points.length; i += bucketSize) {
    const slice = points.slice(i, i + bucketSize);
    const first = slice[0];
    const last = slice[slice.length - 1];
    if (!first || !last) continue;
    const input = slice.reduce((sum, point) => sum + point.input, 0);
    const output = slice.reduce((sum, point) => sum + point.output, 0);
    const cacheRead = slice.reduce((sum, point) => sum + point.cacheRead, 0);
    const cacheWrite = slice.reduce((sum, point) => sum + point.cacheWrite, 0);
    const total = input + output + cacheRead + cacheWrite;
    buckets.push({
      key: `${first.index}-${last.index}`,
      startIndex: first.index,
      endIndex: last.index,
      startTime: first.timestamp,
      endTime: last.timestamp,
      input,
      output,
      cacheRead,
      cacheWrite,
      total,
      cumulative: last.cumulative,
    });
  }

  return buckets;
}

export function buildSessionAnalytics(
  messages: ChatMessage[],
  stats: SessionStats | null = null
): SessionAnalytics {
  const roleCounts = { user: 0, assistant: 0, tool: 0, system: 0 };
  const toolsMap = new Map<string, ToolDistributionItem>();
  const tokenPoints: TokenTimelinePoint[] = [];

  let firstTimestamp: number | null = null;
  let lastTimestamp: number | null = null;
  let lastUsageTimestamp: string | null = null;
  let cumulative = 0;
  let assistantTokenTurns = 0;

  for (let index = 0; index < messages.length; index++) {
    const m = messages[index];
    const roleKey = (m.role || "user").toLowerCase();
    if (roleKey === "user") roleCounts.user++;
    else if (roleKey === "assistant") roleCounts.assistant++;
    else if (roleKey === "tool") roleCounts.tool++;
    else roleCounts.system++;

    let timestamp: number | null = null;
    if (m.timestamp) {
      const parsed = Date.parse(m.timestamp);
      if (!Number.isNaN(parsed)) {
        timestamp = parsed;
        firstTimestamp = firstTimestamp === null ? timestamp : Math.min(firstTimestamp, timestamp);
        lastTimestamp = lastTimestamp === null ? timestamp : Math.max(lastTimestamp, timestamp);
      }
    }

    for (const part of m.content_parts) {
      if (part.type === "tool_use") {
        const rawName = part.tool_name || "Tool";
        const existing = toolsMap.get(rawName);
        if (existing) {
          existing.count++;
        } else {
          toolsMap.set(rawName, {
            key: rawName,
            label: rawName,
            count: 1,
            share: 0,
          });
        }
      }
    }

    // Extract real per-message token usage if available (deduplicated by assistant turn timestamp)
    const usage = m.token_usage;
    if (usage && m.timestamp !== lastUsageTimestamp) {
      const input = usage.input_tokens || 0;
      const output = usage.output_tokens || 0;
      const cacheRead = usage.cache_read_input_tokens || 0;
      const cacheWrite = usage.cache_creation_input_tokens || 0;
      const total = input + output + cacheRead + cacheWrite;
      if (total > 0) {
        lastUsageTimestamp = m.timestamp;
        cumulative += total;
        if (roleKey === "assistant") assistantTokenTurns++;
        tokenPoints.push({
          index,
          timestamp,
          input,
          output,
          cacheRead,
          cacheWrite,
          total,
          cumulative,
        });
      }
    }
  }

  const toolCalls = [...toolsMap.values()].reduce((sum, item) => sum + item.count, 0);
  const toolDistribution = [...toolsMap.values()]
    .map((item) => ({
      ...item,
      share: toolCalls > 0 ? item.count / toolCalls : 0,
    }))
    .sort((a, b) => b.count - a.count);

  let loadedTotals = tokenPoints.reduce(
    (acc, pt) => ({
      input: acc.input + pt.input,
      output: acc.output + pt.output,
      cacheRead: acc.cacheRead + pt.cacheRead,
      cacheWrite: acc.cacheWrite + pt.cacheWrite,
      total: acc.total + pt.total,
    }),
    { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 }
  );

  let tokenTotals = loadedTotals.total > 0 ? loadedTotals : {
    input: stats?.total_input_tokens ?? 0,
    output: stats?.total_output_tokens ?? 0,
    cacheWrite: stats?.total_cache_creation_tokens ?? 0,
    cacheRead: stats?.total_cache_read_tokens ?? 0,
    total: (stats?.total_input_tokens ?? 0) + (stats?.total_output_tokens ?? 0) + (stats?.total_cache_creation_tokens ?? 0) + (stats?.total_cache_read_tokens ?? 0),
  };

  // Fallback synthetic points ONLY if no real per-message tokenPoints exist
  if (tokenPoints.length === 0 && messages.length > 0 && tokenTotals.total > 0) {
    let assistantCount = 0;
    for (const m of messages) {
      if (m.role === "assistant") assistantCount++;
    }
    const turnCount = assistantCount > 0 ? assistantCount : Math.max(1, Math.floor(messages.length / 2));
    const avgInput = Math.round(tokenTotals.input / turnCount);
    const avgOutput = Math.round(tokenTotals.output / turnCount);
    const avgCacheRead = Math.round(tokenTotals.cacheRead / turnCount);
    const avgCacheWrite = Math.round(tokenTotals.cacheWrite / turnCount);

    let runningCum = 0;
    for (let index = 0; index < messages.length; index++) {
      const m = messages[index];
      let ts: number | null = null;
      if (m.timestamp) {
        const p = Date.parse(m.timestamp);
        if (!Number.isNaN(p)) ts = p;
      }
      if (m.role === "assistant" || turnCount === 1) {
        const total = avgInput + avgOutput + avgCacheRead + avgCacheWrite;
        runningCum += total;
        assistantTokenTurns++;
        tokenPoints.push({
          index,
          timestamp: ts,
          input: avgInput,
          output: avgOutput,
          cacheRead: avgCacheRead,
          cacheWrite: avgCacheWrite,
          total,
          cumulative: runningCum,
        });
      }
    }
  }

  const peakTokenPoint =
    tokenPoints.length > 0
      ? tokenPoints.reduce((peak, point) => (point.total > peak.total ? point : peak), tokenPoints[0])
      : null;

  const inputLikeTokens = tokenTotals.input + tokenTotals.cacheRead + tokenTotals.cacheWrite;
  const cacheShare = inputLikeTokens > 0 ? tokenTotals.cacheRead / inputLikeTokens : null;

  const estimatedCost =
    (tokenTotals.input * MODEL_PRICING_DEFAULT.input +
      tokenTotals.output * MODEL_PRICING_DEFAULT.output +
      tokenTotals.cacheWrite * MODEL_PRICING_DEFAULT.cacheWrite +
      tokenTotals.cacheRead * MODEL_PRICING_DEFAULT.cacheRead) /
    1_000_000;

  const averageTokensPerAssistantTurn =
    assistantTokenTurns > 0
      ? loadedTotals.total / assistantTokenTurns
      : stats?.turn_count && stats.turn_count > 0
      ? tokenTotals.total / stats.turn_count
      : null;

  return {
    roleCounts,
    toolCalls,
    toolTypes: toolDistribution.length,
    toolDistribution,
    tokenPoints,
    tokenBuckets: buildTokenBuckets(tokenPoints),
    tokenTotals,
    firstTimestamp,
    lastTimestamp,
    peakTokenPoint,
    averageTokensPerAssistantTurn,
    toolsPerUserTurn: roleCounts.user > 0 ? toolCalls / roleCounts.user : null,
    cacheShare,
    estimatedCost,
  };
}

export function formatAnalyticsTokens(value: number): string {
  const rounded = Math.max(0, Math.round(value));
  if (rounded >= 1_0000_0000) return `${(rounded / 1_0000_0000).toFixed(1)}${t("session.analytics.hundredMillion")}`;
  if (rounded >= 1_0000) return `${(rounded / 1_0000).toFixed(1)}${t("session.analytics.tenThousand")}`;
  return formatNumber(rounded);
}

export function formatTimeOnly(ts: number | null): string {
  if (!ts) return "—";
  const date = new Date(ts);
  const h = String(date.getHours()).padStart(2, "0");
  const m = String(date.getMinutes()).padStart(2, "0");
  return `${h}:${m}`;
}

export function formatDurationText(ms: number): string {
  if (ms <= 0) return "—";
  const totalSec = Math.floor(ms / 1000);
  const hours = Math.floor(totalSec / 3600);
  const mins = Math.floor((totalSec % 3600) / 60);
  const secs = totalSec % 60;
  if (hours > 0) return `${hours}h ${mins}m`;
  if (mins > 0) return `${mins} min`;
  return `${secs}s`;
}
