<script setup lang="ts">
import { computed, ref } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";
import { formatNumber } from "../utils/format";
import type { ChatMessage, SessionStats } from "../types/session";
import {
  buildSessionAnalytics,
  formatAnalyticsTokens,
  formatDurationText,
  formatTimeOnly,
  type TokenTimelineBucket,
} from "../utils/sessionAnalytics";

const props = defineProps<{
  displayName: string;
  messages: ChatMessage[];
  stats: SessionStats | null;
}>();

const emit = defineEmits<{
  close: [];
}>();

const analytics = computed(() => buildSessionAnalytics(props.messages, props.stats));

const activeBucketIndex = ref<number | null>(null);
const hoverBucketIndex = ref<number | null>(null);

const TOOL_COLORS = [
  "#3b82f6",
  "#10b981",
  "#f59e0b",
  "#8b5cf6",
  "#ef4444",
  "#06b6d4",
  "#06b6d4",
  "#ec4899",
  "#f97316",
  "#64748b",
];

function getToolGlyphConfig(name: string) {
  const n = name.toLowerCase();
  if (n.includes("bash") || n.includes("terminal") || n.includes("shell") || n.includes("command")) {
    return { icon: "terminal", bg: "rgba(59, 130, 246, 0.12)", color: "#3b82f6" };
  }
  if (n.includes("task") || n.includes("todo") || n.includes("plan")) {
    return { icon: "check-square", bg: "rgba(16, 185, 129, 0.12)", color: "#10b981" };
  }
  if (n.includes("agent") || n.includes("bot") || n.includes("subagent")) {
    return { icon: "bot", bg: "rgba(245, 158, 11, 0.12)", color: "#f59e0b" };
  }
  if (n.includes("read") || n.includes("file") || n.includes("view")) {
    return { icon: "file-text", bg: "rgba(239, 68, 68, 0.12)", color: "#ef4444" };
  }
  if (n.includes("edit") || n.includes("write") || n.includes("patch") || n.includes("replace")) {
    return { icon: "edit", bg: "rgba(6, 182, 212, 0.12)", color: "#06b6d4" };
  }
  if (n.includes("ask") || n.includes("question") || n.includes("user")) {
    return { icon: "help-circle", bg: "rgba(168, 85, 247, 0.12)", color: "#a855f7" };
  }
  if (n.includes("search") || n.includes("grep") || n.includes("glob") || n.includes("find")) {
    return { icon: "search", bg: "rgba(99, 102, 241, 0.12)", color: "#6366f1" };
  }
  return { icon: "wrench", bg: "rgba(236, 72, 153, 0.12)", color: "#ec4899" };
}

// Tool list visible & hidden group (limit 7 + 其他工具)
const visibleTools = computed(() => {
  const list = analytics.value.toolDistribution;
  const visible = list.slice(0, 7);
  const hidden = list.slice(7);
  const hiddenCount = hidden.reduce((sum, item) => sum + item.count, 0);

  if (hiddenCount > 0) {
    return [
      ...visible,
      {
        key: "other-tools",
        label: t("common.sessionAnalytics.otherTools"),
        count: hiddenCount,
        share: analytics.value.toolCalls > 0 ? hiddenCount / analytics.value.toolCalls : 0,
      },
    ];
  }
  return visible;
});

// SVG Token Chart calculations
const chartWidth = 640;
const chartHeight = 226;
const chartTop = 18;
const chartBottom = 170;
const chartLeft = 48;
const chartRight = 622;
const innerChartHeight = chartBottom - chartTop;
const innerChartWidth = chartRight - chartLeft;

const buckets = computed(() => analytics.value.tokenBuckets);

const maxValue = computed(() => {
  const max = Math.max(...buckets.value.map((b) => b.total), 1);
  return max;
});

const maxCumulative = computed(() => {
  const max = Math.max(...buckets.value.map((b) => b.cumulative), 1);
  return max;
});

const step = computed(() => innerChartWidth / Math.max(buckets.value.length, 1));
const barWidth = computed(() => Math.max(3, Math.min(13, step.value * 0.56)));

const activeIndex = computed(() => {
  if (buckets.value.length === 0) return -1;
  return Math.min(
    activeBucketIndex.value ?? buckets.value.length - 1,
    buckets.value.length - 1
  );
});

const activeBucket = computed(() => {
  if (activeIndex.value >= 0 && buckets.value[activeIndex.value]) {
    return buckets.value[activeIndex.value];
  }
  return null;
});

const activeX = computed(() => {
  if (!activeBucket.value) return null;
  return chartLeft + step.value * activeIndex.value + step.value / 2;
});

const activeLineY = computed(() => {
  if (!activeBucket.value) return null;
  return chartBottom - (activeBucket.value.cumulative / maxCumulative.value) * innerChartHeight;
});

const yTicks = computed(() => {
  return [1, 0.5, 0].map((ratio) => ({
    key: ratio,
    label: ratio === 0 ? "0" : formatAnalyticsTokens(maxValue.value * ratio),
    y: chartBottom - ratio * innerChartHeight,
  }));
});

const xTickIndexes = computed(() => {
  if (buckets.value.length === 0) return [];
  const mid = Math.floor((buckets.value.length - 1) / 2);
  const set = new Set([0, mid, buckets.value.length - 1]);
  return [...set].filter((i) => i >= 0);
});

const linePoints = computed(() => {
  return buckets.value
    .map((bucket, index) => {
      const x = chartLeft + step.value * index + step.value / 2;
      const y = chartBottom - (bucket.cumulative / maxCumulative.value) * innerChartHeight;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
});

function timeRangeLabel(start: number | null, end: number | null): string {
  if (start === null && end === null) return "—";
  if (start !== null && end !== null && start !== end) {
    return `${formatTimeOnly(start)} – ${formatTimeOnly(end)}`;
  }
  const only = start ?? end;
  return only === null ? "—" : formatTimeOnly(only);
}

function getBucketBars(bucket: TokenTimelineBucket, index: number) {
  const x = chartLeft + step.value * index + (step.value - barWidth.value) / 2;
  let y = chartBottom;
  const scale = innerChartHeight / maxValue.value;
  const inputHeight = bucket.input > 0 ? Math.max(1, bucket.input * scale) : 0;
  const outputHeight = bucket.output > 0 ? Math.max(1, bucket.output * scale) : 0;
  const cacheHeight =
    bucket.cacheRead + bucket.cacheWrite > 0
      ? Math.max(1, (bucket.cacheRead + bucket.cacheWrite) * scale)
      : 0;

  y -= cacheHeight;
  const cacheY = y;
  y -= outputHeight;
  const outputY = y;
  y -= inputHeight;
  const inputY = y;

  return { x, inputHeight, inputY, outputHeight, outputY, cacheHeight, cacheY };
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="analytics-modal-container">
      <!-- Modal Header -->
      <div class="analytics-header">
        <div class="header-titles">
          <h2 class="modal-title">{{ t("common.sessionAnalytics.title") }}</h2>
          <p class="modal-subtitle truncate">{{ t("common.sessionAnalytics.subtitle", { name: displayName }) }}</p>
        </div>
        <button class="close-btn" type="button" :title="t('dialogs.common.close')" @click="emit('close')">
          <SvgIcon name="x" :size="18" />
        </button>
      </div>

      <!-- Modal Body -->
      <div class="analytics-body">
        <!-- Stat Tiles Grid -->
        <div class="stat-tiles-grid">
          <!-- Tile 1: Tokens -->
          <div class="stat-tile tile-blue">
            <div class="tile-header">
              <div class="tile-icon-box">
                <SvgIcon name="activity" :size="16" />
              </div>
              <span class="tile-label">{{ t("common.sessionAnalytics.totalTokens") }}</span>
            </div>
            <div class="tile-value">{{ formatAnalyticsTokens(analytics.tokenTotals.total) }}</div>
            <div class="tile-detail">
              {{ t("common.sessionAnalytics.analyzedMessages", { count: analytics.roleCounts.user + analytics.roleCounts.assistant }) }}
            </div>
          </div>

          <!-- Tile 2: Tool Calls -->
          <div class="stat-tile tile-green">
            <div class="tile-header">
              <div class="tile-icon-box">
                <SvgIcon name="wrench" :size="16" />
              </div>
              <span class="tile-label">{{ t("common.sessionAnalytics.toolCalls") }}</span>
            </div>
            <div class="tile-value">{{ formatNumber(analytics.toolCalls) }}</div>
            <div class="tile-detail">{{ t("common.sessionAnalytics.toolTypeCount", { count: analytics.toolTypes }) }}</div>
          </div>

          <!-- Tile 3: Duration -->
          <div class="stat-tile tile-amber">
            <div class="tile-header">
              <div class="tile-icon-box">
                <SvgIcon name="clock" :size="16" />
              </div>
              <span class="tile-label">{{ t("common.sessionAnalytics.timeSpan") }}</span>
            </div>
            <div class="tile-value">
              {{ analytics.firstTimestamp && analytics.lastTimestamp ? formatDurationText(analytics.lastTimestamp - analytics.firstTimestamp) : '—' }}
            </div>
            <div class="tile-detail">{{ t("common.sessionAnalytics.firstToLast") }}</div>
          </div>

          <!-- Tile 4: Avg Tokens -->
          <div class="stat-tile tile-pink">
            <div class="tile-header">
              <div class="tile-icon-box">
                <SvgIcon name="gauge" :size="16" />
              </div>
              <span class="tile-label">{{ t("common.sessionAnalytics.averageTokens") }}</span>
            </div>
            <div class="tile-value">
              {{ analytics.averageTokensPerAssistantTurn ? formatAnalyticsTokens(analytics.averageTokensPerAssistantTurn) : "—" }}
            </div>
            <div class="tile-detail">{{ t("common.sessionAnalytics.perAssistantTurn") }}</div>
          </div>
        </div>

        <!-- Tool Distribution Section -->
        <div class="analytics-card">
          <div class="card-header-row">
            <div class="card-header-titles">
              <h3 class="card-title">{{ t("common.sessionAnalytics.toolDistribution") }}</h3>
              <span class="card-hint">{{ t("common.sessionAnalytics.toolDistributionHint") }}</span>
            </div>
            <div class="card-header-icon">
              <SvgIcon name="clock" :size="18" />
            </div>
          </div>

          <div v-if="visibleTools.length === 0" class="empty-tools-hint">
            {{ t("common.sessionAnalytics.noToolCalls") }}
          </div>

          <div v-else class="tool-list">
            <div
              v-for="(item, index) in visibleTools"
              :key="item.key"
              class="tool-row"
            >
              <div
                class="tool-glyph-badge"
                :style="{
                  backgroundColor: getToolGlyphConfig(item.label).bg,
                  color: getToolGlyphConfig(item.label).color,
                }"
              >
                <SvgIcon :name="getToolGlyphConfig(item.label).icon" :size="13" />
              </div>
              <span class="tool-name">{{ item.label }}</span>
              <div class="tool-row-track">
                <div
                  class="tool-row-bar"
                  :style="{
                    width: `${Math.max(4, Math.round(item.share * 100))}%`,
                    backgroundColor: TOOL_COLORS[index % TOOL_COLORS.length],
                  }"
                ></div>
              </div>
              <strong class="tool-count">{{ formatNumber(item.count) }}</strong>
              <small class="tool-share">{{ Math.round(item.share * 100) }}%</small>
            </div>
          </div>
        </div>

        <!-- Token Timeline Chart (Interactive SVG) -->
        <div class="analytics-card">
          <div class="card-header-row">
            <div class="card-header-titles">
              <h3 class="card-title">{{ t("common.sessionAnalytics.tokenTimeline") }}</h3>
              <span class="card-hint">{{ t("common.sessionAnalytics.tokenTimelineHint") }}</span>
            </div>
            <div class="card-header-icon">
              <SvgIcon name="trending-up" :size="18" />
            </div>
          </div>

          <div class="token-chart-container">
            <svg :viewBox="`0 0 ${chartWidth} ${chartHeight}`" class="token-svg">
              <defs>
                <linearGradient id="token-fill" x1="0" x2="0" y1="0" y2="1">
                  <stop offset="0%" stop-color="var(--color-primary, #3b82f6)" stop-opacity="0.22" />
                  <stop offset="100%" stop-color="var(--color-primary, #3b82f6)" stop-opacity="0.02" />
                </linearGradient>
              </defs>

              <!-- Base Axis & Grid Lines -->
              <line class="axis-line" :x1="chartLeft" :x2="chartRight" :y1="chartBottom" :y2="chartBottom" />
              <g v-for="tick in yTicks" :key="tick.key">
                <line class="grid-line" :x1="chartLeft" :x2="chartRight" :y1="tick.y" :y2="tick.y" />
                <text class="tick-label" text-anchor="end" :x="chartLeft - 8" :y="tick.y + 3">
                  {{ tick.label }}
                </text>
              </g>

              <!-- X Ticks -->
              <g v-for="idx in xTickIndexes" :key="idx">
                <line
                  class="x-tick"
                  :x1="chartLeft + step * idx + step / 2"
                  :x2="chartLeft + step * idx + step / 2"
                  :y1="chartBottom"
                  :y2="chartBottom + 5"
                />
                <text
                  class="x-label"
                  text-anchor="middle"
                  :x="chartLeft + step * idx + step / 2"
                  :y="chartBottom + 20"
                >
                  {{ timeRangeLabel(buckets[idx]?.startTime ?? null, buckets[idx]?.endTime ?? null) }}
                </text>
              </g>

              <!-- Gradient Area under Cumulative Line -->
              <polyline
                class="token-area-fill"
                fill="url(#token-fill)"
                :points="`${chartLeft},${chartBottom} ${linePoints} ${chartRight},${chartBottom}`"
              />

              <!-- Stacked Bars -->
              <g
                v-for="(b, idx) in buckets"
                :key="b.key"
                class="bucket-bars-group"
                :class="{ 'is-active': idx === activeIndex }"
              >
                <rect
                  v-if="getBucketBars(b, idx).inputHeight > 0"
                  class="bar-input"
                  :x="getBucketBars(b, idx).x"
                  :y="getBucketBars(b, idx).inputY"
                  :width="barWidth"
                  :height="getBucketBars(b, idx).inputHeight"
                  rx="2"
                />
                <rect
                  v-if="getBucketBars(b, idx).outputHeight > 0"
                  class="bar-output"
                  :x="getBucketBars(b, idx).x"
                  :y="getBucketBars(b, idx).outputY"
                  :width="barWidth"
                  :height="getBucketBars(b, idx).outputHeight"
                  rx="2"
                />
                <rect
                  v-if="getBucketBars(b, idx).cacheHeight > 0"
                  class="bar-cache"
                  :x="getBucketBars(b, idx).x"
                  :y="getBucketBars(b, idx).cacheY"
                  :width="barWidth"
                  :height="getBucketBars(b, idx).cacheHeight"
                  rx="2"
                />
              </g>

              <!-- Cumulative Line -->
              <polyline class="cumulative-line" :points="linePoints" />

              <!-- Active Marker Circle -->
              <g v-if="activeX !== null && activeLineY !== null" class="active-marker">
                <line :x1="activeX" :x2="activeX" :y1="chartTop" :y2="chartBottom" />
                <circle :cx="activeX" :cy="activeLineY" r="4" />
              </g>
            </svg>

            <!-- Hotspot Click Overlay -->
            <div class="hotspots-overlay">
              <button
                v-for="(b, idx) in buckets"
                :key="`hs-${b.key}`"
                type="button"
                class="hotspot-btn"
                :class="{ active: idx === activeIndex }"
                :style="{
                  left: `${((chartLeft + step * idx) / chartWidth) * 100}%`,
                  width: `${(step / chartWidth) * 100}%`,
                }"
                @click="activeBucketIndex = idx"
                @mouseenter="hoverBucketIndex = idx"
                @mouseleave="hoverBucketIndex = null"
              />
            </div>
          </div>

          <!-- Active Bucket Details Panel -->
          <div v-if="activeBucket" class="active-bucket-details">
            <div class="b-detail-item">
              <small>{{ t("common.sessionAnalytics.selectedRange") }}</small>
              <strong>{{ timeRangeLabel(activeBucket.startTime, activeBucket.endTime) }}</strong>
            </div>
            <div class="b-detail-item">
              <small>{{ t("common.sessionAnalytics.rangeTotal") }}</small>
              <strong>{{ formatAnalyticsTokens(activeBucket.total) }}</strong>
            </div>
            <div class="b-detail-item">
              <small>{{ t("common.sessionAnalytics.inputTokens") }}</small>
              <strong>{{ formatAnalyticsTokens(activeBucket.input) }}</strong>
            </div>
            <div class="b-detail-item">
              <small>{{ t("common.sessionAnalytics.outputTokens") }}</small>
              <strong>{{ formatAnalyticsTokens(activeBucket.output) }}</strong>
            </div>
            <div class="b-detail-item">
              <small>{{ t("common.sessionAnalytics.cacheTokens") }}</small>
              <strong>{{ formatAnalyticsTokens(activeBucket.cacheRead + activeBucket.cacheWrite) }}</strong>
            </div>
            <div class="b-detail-item">
              <small>{{ t("common.sessionAnalytics.cumulativeToHere") }}</small>
              <strong>{{ formatAnalyticsTokens(activeBucket.cumulative) }}</strong>
            </div>
          </div>

          <!-- Token Legend -->
          <div class="token-legend">
            <span class="legend-item"><span class="legend-dot input-dot"></span>{{ t("common.sessionAnalytics.inputTokens") }}</span>
            <span class="legend-item"><span class="legend-dot output-dot"></span>{{ t("common.sessionAnalytics.outputTokens") }}</span>
            <span class="legend-item"><span class="legend-dot cache-dot"></span>{{ t("common.sessionAnalytics.cacheTokens") }}</span>
            <span class="legend-item"><span class="legend-line"></span>{{ t("common.sessionAnalytics.cumulativeLegend") }}</span>
          </div>
        </div>

        <!-- Key Insights Grid -->
        <div class="insights-grid">
          <div class="insight-item">
            <div class="insight-icon">
              <SvgIcon name="sparkles" :size="14" />
            </div>
            <div class="insight-copy">
              <span class="insight-label">{{ t("common.sessionAnalytics.peakTurn") }}</span>
              <strong class="insight-val">
                {{ analytics.peakTokenPoint ? t("common.sessionAnalytics.peakTurnValue", { index: analytics.peakTokenPoint.index + 1, tokens: formatAnalyticsTokens(analytics.peakTokenPoint.total) }) : '—' }}
              </strong>
            </div>
          </div>

          <div class="insight-item">
            <div class="insight-icon">
              <SvgIcon name="terminal" :size="14" />
            </div>
            <div class="insight-copy">
              <span class="insight-label">{{ t("common.sessionAnalytics.topTool") }}</span>
              <strong class="insight-val">
                {{ analytics.toolDistribution.length > 0 ? `${analytics.toolDistribution[0].label} (${Math.round(analytics.toolDistribution[0].share * 100)}%)` : t("common.sessionAnalytics.none") }}
              </strong>
            </div>
          </div>

          <div class="insight-item">
            <div class="insight-icon">
              <SvgIcon name="refresh-cw" :size="14" />
            </div>
            <div class="insight-copy">
              <span class="insight-label">{{ t("common.sessionAnalytics.cacheHitRate") }}</span>
              <strong class="insight-val">
                {{ analytics.cacheShare !== null ? `${Math.round(analytics.cacheShare * 100)}%` : '—' }}
              </strong>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-backdrop {
  padding: 24px;
  animation: fadeIn 0.2s ease-out;
}

.analytics-modal-container {
  width: 100%;
  max-width: 720px;
  max-height: 90vh;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl, 16px);
  box-shadow: 0 20px 40px -10px rgba(0, 0, 0, 0.28);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: scaleUp 0.22s cubic-bezier(0.16, 1, 0.3, 1);
}

.analytics-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  padding: 20px 24px 16px;
  border-bottom: 1px solid var(--color-border);
}

.modal-title {
  margin: 0;
  font-size: var(--text-lg, 18px);
  font-weight: 600;
  color: var(--color-text);
}

.modal-subtitle {
  margin: 4px 0 0;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
  max-width: 540px;
}

.close-btn {
  background: transparent;
  border: none;
  color: var(--color-text-muted);
  cursor: pointer;
  padding: 6px;
  border-radius: var(--radius-md, 6px);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--transition-fast, 0.15s);
}

.close-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.analytics-body {
  padding: 20px 24px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* Stat Tiles Grid */
.stat-tiles-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 12px;
}

.stat-tile {
  padding: 14px 16px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 12px);
  display: flex;
  flex-direction: column;
  gap: 4px;
  transition: transform 0.15s ease;
}

.stat-tile:hover {
  transform: translateY(-1px);
}

.tile-header {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tile-icon-box {
  width: 30px;
  height: 30px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.tile-blue .tile-icon-box {
  background: rgba(59, 130, 246, 0.12);
  color: #3b82f6;
}

.tile-green .tile-icon-box {
  background: rgba(16, 185, 129, 0.12);
  color: #10b981;
}

.tile-amber .tile-icon-box {
  background: rgba(245, 158, 11, 0.12);
  color: #f59e0b;
}

.tile-pink .tile-icon-box {
  background: rgba(236, 72, 153, 0.12);
  color: #ec4899;
}

.tile-label {
  font-size: 13px;
  color: var(--color-text-secondary);
  font-weight: 500;
}

.tile-value {
  font-size: 22px;
  font-weight: 700;
  color: var(--color-text);
  margin-top: 2px;
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.5px;
}

.tile-detail {
  font-size: 12px;
  color: var(--color-text-muted);
}

/* Key Insights Grid */
.insights-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
}

.insight-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: 10px;
}

.insight-icon {
  width: 26px;
  height: 26px;
  border-radius: 6px;
  background: var(--color-bg);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-primary, #3b82f6);
  flex-shrink: 0;
}

.insight-copy {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.insight-label {
  font-size: 11px;
  color: var(--color-text-muted);
}

.insight-val {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Analytics Card */
.analytics-card {
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 12px);
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.card-header-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}

.card-header-titles {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.card-header-icon {
  color: var(--color-text-muted);
  opacity: 0.6;
}

.card-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text);
}

.card-hint {
  font-size: 12px;
  color: var(--color-text-muted);
}

/* Tool Distribution */
.empty-tools-hint {
  font-size: 13px;
  color: var(--color-text-muted);
  text-align: center;
  padding: 16px 0;
}

.tool-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.tool-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
}

.tool-glyph-badge {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.tool-name {
  width: 120px;
  font-weight: 500;
  color: var(--color-text);
  font-family: var(--font-mono, monospace);
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
}

.tool-row-track {
  flex: 1;
  height: 8px;
  background: var(--color-bg);
  border-radius: 4px;
  overflow: hidden;
}

.tool-row-bar {
  height: 100%;
  border-radius: 4px;
  transition: width 0.4s ease-out;
}

.tool-count {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  font-variant-numeric: tabular-nums;
  width: 40px;
  text-align: right;
}

.tool-share {
  font-size: 12px;
  color: var(--color-text-muted);
  width: 36px;
  text-align: right;
}

/* Token Chart SVG */
.token-chart-container {
  position: relative;
  width: 100%;
}

.token-svg {
  width: 100%;
  height: auto;
  display: block;
}

.axis-line {
  stroke: var(--color-border);
  stroke-width: 1;
}

.grid-line {
  stroke: var(--color-border);
  stroke-width: 1;
  stroke-dasharray: 3 3;
  opacity: 0.5;
}

.x-tick {
  stroke: var(--color-border);
  stroke-width: 1;
}

.tick-label,
.x-label {
  font-size: 10px;
  fill: var(--color-text-muted);
  font-family: var(--font-sans, sans-serif);
}

.bar-input {
  fill: #3b82f6;
}

.bar-output {
  fill: #10b981;
}

.bar-cache {
  fill: #f59e0b;
}

.cumulative-line {
  fill: none;
  stroke: var(--color-primary, #3b82f6);
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.active-marker line {
  stroke: var(--color-primary, #3b82f6);
  stroke-width: 1.5;
  stroke-dasharray: 2 2;
}

.active-marker circle {
  fill: var(--color-primary, #3b82f6);
  stroke: var(--color-bg);
  stroke-width: 2;
}

.hotspots-overlay {
  position: absolute;
  top: 18px;
  bottom: 56px;
  left: 0;
  right: 0;
  display: flex;
}

.hotspot-btn {
  background: transparent;
  border: none;
  cursor: pointer;
}

.hotspot-btn:hover,
.hotspot-btn.active {
  background: rgba(59, 130, 246, 0.06);
}

/* Active Bucket Details Panel */
.active-bucket-details {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 8px;
  padding: 10px 12px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: 8px;
  margin-top: 10px;
}

.b-detail-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.b-detail-item small {
  font-size: 10px;
  color: var(--color-text-muted);
}

.b-detail-item strong {
  font-size: 12px;
  color: var(--color-text);
  font-variant-numeric: tabular-nums;
}

/* Token Legend */
.token-legend {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 16px;
  margin-top: 10px;
  font-size: 11px;
  color: var(--color-text-muted);
}

.legend-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.legend-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
}

.input-dot {
  background: #3b82f6;
}

.output-dot {
  background: #10b981;
}

.cache-dot {
  background: #f59e0b;
}

.legend-line {
  width: 14px;
  height: 2px;
  background: var(--color-primary, #3b82f6);
  border-radius: 1px;
  display: inline-block;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes scaleUp {
  from { transform: scale(0.96); opacity: 0; }
  to { transform: scale(1); opacity: 1; }
}
</style>
