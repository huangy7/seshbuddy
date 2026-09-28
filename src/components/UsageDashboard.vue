<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import type { UsageRecord } from "../types/session";
import { useSessions } from "../composables/useSessions";
import { t } from "../i18n";
import { resolveVisibleCliIds, toggleVisibleCli, type CliFilterState } from "../composables/cliFilter";
import { isCliId, type CliId } from "../types/cli";
import { formatCost, formatCostCompact, formatNumber } from "../utils/format";
import { useBlockedFolders, isPathBlocked } from "../composables/useBlockedFolders";
import { useUsageStats } from "../composables/useUsageStats";
import { usePricingCatalog } from "../composables/usePricingCatalog";
import PricingCatalogDialog from "./PricingCatalogDialog.vue";
import {
  calculateModelBreakdown,
  calculateProjectBreakdown,
  getRecordCost,
} from "../utils/usageDrilldown";

const emit = defineEmits<{ close: [] }>();
const { blockedFolders, loadBlockedFolders } = useBlockedFolders();
defineProps<{ initialCliId?: CliId }>();
const { cliOptions, cliSessionCounts } = useSessions();
const { catalog: pricingCatalogStore, status: pricingStatus } = usePricingCatalog();
const showPricingCatalog = ref(false);
const USAGE_CLI_FILTER_STORAGE_KEY = "seshbuddy-usage-cli-filter";
const USAGE_MAXIMIZED_STORAGE_KEY = "seshbuddy-usage-maximized";

function loadPersistedFilter(): CliFilterState {
  try {
    const raw = localStorage.getItem(USAGE_CLI_FILTER_STORAGE_KEY);
    if (!raw) return { mode: "all" };
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object") {
      if (parsed.mode === "all") return { mode: "all" };
      if (parsed.mode === "custom" && Array.isArray(parsed.cliIds)) {
        return {
          mode: "custom",
          cliIds: parsed.cliIds.filter((id: unknown): id is CliId => typeof id === "string" && isCliId(id)),
        };
      }
    }
  } catch {
    // ignore parse errors
  }
  return { mode: "all" };
}

const supportedCliOptions = computed(() =>
  cliOptions.value.filter((cli) => cli.supportsUsageStats),
);
const supportedCliIds = computed(() => supportedCliOptions.value.map((cli) => cli.id));
const usageCliFilter = ref<CliFilterState>(loadPersistedFilter());
const usageCliIds = computed(() => resolveVisibleCliIds(usageCliFilter.value, supportedCliIds.value));
const isCliSelected = (cliId: CliId) => usageCliIds.value.includes(cliId);

function toggleAllCli() {
  if (usageCliFilter.value.mode === "all") {
    usageCliFilter.value = { mode: "custom", cliIds: [] };
  } else {
    usageCliFilter.value = { mode: "all" };
  }
}

function toggleCli(cliId: CliId) {
  usageCliFilter.value = toggleVisibleCli(
    usageCliFilter.value,
    supportedCliIds.value,
    cliId,
  );
}

// Window Maximize Mode
const isMaximized = ref(localStorage.getItem(USAGE_MAXIMIZED_STORAGE_KEY) === "true");

function toggleMaximize() {
  isMaximized.value = !isMaximized.value;
  localStorage.setItem(USAGE_MAXIMIZED_STORAGE_KEY, String(isMaximized.value));
}

// SWR Stats Store Integration
const { recordsByCli, loading, loadError, fetchUsage } = useUsageStats();
const timeRange = ref<"7d" | "30d" | "all">("30d");

function handleRefresh() {
  void fetchUsage(usageCliIds.value, true);
}

// Blocked folders inclusion for usage stats calculation
const includedBlockedFolders = ref<Set<string>>(new Set());

const isAllBlockedFoldersIncluded = computed(() =>
  blockedFolders.value.length > 0 &&
  blockedFolders.value.every((f) => includedBlockedFolders.value.has(f))
);

function toggleIncludedFolder(folder: string) {
  const next = new Set(includedBlockedFolders.value);
  if (next.has(folder)) {
    next.delete(folder);
  } else {
    next.add(folder);
  }
  includedBlockedFolders.value = next;
}

function toggleAllBlockedFolders() {
  if (isAllBlockedFoldersIncluded.value) {
    includedBlockedFolders.value = new Set();
  } else {
    includedBlockedFolders.value = new Set(blockedFolders.value);
  }
}

function isRecordBlocked(projectPath: string): boolean {
  if (!projectPath || blockedFolders.value.length === 0) return false;
  for (const b of blockedFolders.value) {
    if (isPathBlocked(projectPath, [b])) {
      if (!includedBlockedFolders.value.has(b)) {
        return true;
      }
    }
  }
  return false;
}

const selectedRecords = computed(() => {
  const list: UsageRecord[] = [];
  for (const cliId of usageCliIds.value) {
    const arr = recordsByCli.value.get(cliId);
    if (arr) list.push(...arr);
  }
  return list;
});

// Filter records by time range and blocked folders
const filteredRecords = computed(() => {
  let list = selectedRecords.value;
  if (timeRange.value !== "all") {
    const now = new Date();
    const days = timeRange.value === "7d" ? 7 : 30;
    const cutoff = new Date(now.getTime() - days * 86400000);
    const cutoffStr = cutoff.toISOString().slice(0, 10);
    list = list.filter((r) => r.date >= cutoffStr);
  }
  return list.filter((r) => !isRecordBlocked(r.project));
});

const usageSupported = computed(() => supportedCliOptions.value.length > 0);
const usageViewState = computed<"loading" | "unsupported" | "error" | "empty" | "data">(() => {
  if (loading.value) return "loading";
  if (!usageSupported.value) return "unsupported";
  if (loadError.value && filteredRecords.value.length === 0) return "error";
  if (filteredRecords.value.length === 0) return "empty";
  return "data";
});

// Summary stats
const totalTokens = computed(() =>
  filteredRecords.value.reduce(
    (sum, r) => sum + r.input_tokens + r.output_tokens + r.cache_creation_tokens + r.cache_read_tokens,
    0,
  ),
);

const totalCost = computed(() => {
  void pricingCatalogStore.value;
  return filteredRecords.value.reduce((sum, r) => sum + getRecordCost(r), 0);
});

const totalTurns = computed(() => filteredRecords.value.length);

const totalDuration = computed(() =>
  filteredRecords.value.reduce((sum, r) => sum + (r.duration_ms || 0), 0),
);

// Daily aggregation for chart
const dailyStats = computed(() => {
  void pricingCatalogStore.value;
  const map = new Map<string, { tokens: number; cost: number; turns: number }>();
  for (const r of filteredRecords.value) {
    const existing = map.get(r.date) || { tokens: 0, cost: 0, turns: 0 };
    existing.tokens += r.input_tokens + r.output_tokens + r.cache_creation_tokens + r.cache_read_tokens;
    existing.cost += getRecordCost(r);
    existing.turns += 1;
    map.set(r.date, existing);
  }
  const entries = Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  return entries.map(([date, data]) => ({ date, ...data }));
});

const maxDailyCost = computed(() =>
  Math.max(...dailyStats.value.map((d) => d.cost), 0.01),
);

// Bidirectional drilldown breakdowns
const DEFAULT_DRILLDOWN_LIMIT = 10;
const showAllModels = ref(false);
const showAllProjects = ref(false);

const modelBreakdowns = computed(() => {
  void pricingCatalogStore.value;
  return calculateModelBreakdown(filteredRecords.value);
});
const projectBreakdowns = computed(() => {
  void pricingCatalogStore.value;
  return calculateProjectBreakdown(filteredRecords.value);
});

const visibleModelBreakdowns = computed(() =>
  showAllModels.value ? modelBreakdowns.value : modelBreakdowns.value.slice(0, DEFAULT_DRILLDOWN_LIMIT),
);
const visibleProjectBreakdowns = computed(() =>
  showAllProjects.value ? projectBreakdowns.value : projectBreakdowns.value.slice(0, DEFAULT_DRILLDOWN_LIMIT),
);

const expandedModelKeys = ref<Set<string>>(new Set());
const expandedProjectKeys = ref<Set<string>>(new Set());

function toggleModelExpand(model: string) {
  const next = new Set(expandedModelKeys.value);
  if (next.has(model)) {
    next.delete(model);
  } else {
    next.add(model);
  }
  expandedModelKeys.value = next;
}

function toggleProjectExpand(project: string) {
  const next = new Set(expandedProjectKeys.value);
  if (next.has(project)) {
    next.delete(project);
  } else {
    next.add(project);
  }
  expandedProjectKeys.value = next;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return (n / 1_000_000_000).toFixed(1) + "B";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return n.toString();
}

function formatDuration(ms: number): string {
  if (ms < 60000) return Math.round(ms / 1000) + "s";
  if (ms < 3600000) return Math.round(ms / 60000) + "m";
  const h = Math.floor(ms / 3600000);
  const m = Math.round((ms % 3600000) / 60000);
  return h + "h " + m + "m";
}

function formatDate(date: string): string {
  return date.slice(5); // MM-DD
}

function rangeLabel(range: "7d" | "30d" | "all"): string {
  if (range === "7d") return t("session.usageDashboard.range7d");
  if (range === "30d") return t("session.usageDashboard.range30d");
  return t("session.usageDashboard.all");
}

function getModelColor(model: string): string {
  const lower = model.toLowerCase();
  if (lower.includes("haiku")) return "var(--color-success, #22c55e)";
  if (lower.includes("sonnet")) return "var(--color-primary, #3b82f6)";
  if (lower.includes("opus")) return "#a855f7";
  if (lower.includes("gemini") || lower.includes("flash")) return "#06b6d4";
  return "var(--color-text-muted)";
}

onMounted(async () => {
  void loadBlockedFolders()?.catch?.((err) => {
    console.error("loadBlockedFolders failed:", err);
  });
  await fetchUsage(usageCliIds.value);
});

watch(
  usageCliFilter,
  (val) => {
    localStorage.setItem(USAGE_CLI_FILTER_STORAGE_KEY, JSON.stringify(val));
    void fetchUsage(usageCliIds.value);
  },
  { deep: true },
);

const usedModelNames = computed(() => {
  const set = new Set<string>();
  for (const r of filteredRecords.value) {
    if (r.model) set.add(r.model);
  }
  return Array.from(set);
});

const totalSessionsCount = computed(() => {
  return supportedCliOptions.value.reduce(
    (sum, cli) => sum + (cliSessionCounts.value?.[cli.id] ?? 0),
    0,
  );
});

watch(supportedCliIds, () => {
  void fetchUsage(usageCliIds.value);
});
</script>

<template>
  <div class="modal-backdrop" :class="{ 'modal-backdrop--maximized': isMaximized }">
    <div class="usage-window" :class="{ 'is-maximized': isMaximized, 'usage-window-maximized': isMaximized }">
      <div class="usage-header">
        <div class="usage-title-cluster">
          <h2 class="usage-title">
            <SvgIcon name="bar-chart-2" :size="18" />
            {{ t("session.usageDashboard.title") }}
          </h2>
          <div class="range-bar header-range-bar" role="group" :aria-label="t('session.usageDashboard.rangeAria')">
            <button
              v-for="range in (['7d', '30d', 'all'] as const)"
              :key="range"
              type="button"
              class="range-btn"
              :class="{ active: timeRange === range }"
              @click="timeRange = range"
            >
              {{ rangeLabel(range) }}
            </button>
          </div>
        </div>
        <div class="usage-header-actions">
          <button
            type="button"
            class="icon-btn icon-btn--lg maximize-btn"
            :title="isMaximized ? t('session.usageDashboard.restore') : t('session.usageDashboard.maximize')"
            :aria-label="isMaximized ? t('session.usageDashboard.restore') : t('session.usageDashboard.maximize')"
            @click="toggleMaximize"
          >
            <SvgIcon :name="isMaximized ? 'minimize-2' : 'maximize-2'" :size="18" />
          </button>
          <button
            type="button"
            class="close-btn icon-btn icon-btn--lg"
            :title="t('dialogs.common.close')"
            :aria-label="t('dialogs.common.close')"
            @click="emit('close')"
          >
            <SvgIcon name="x" :size="18" />
          </button>
        </div>
      </div>

      <div class="usage-body">
        <div class="usage-controls-bar">
          <div class="usage-controls-top">
            <div class="usage-controls-meta">
              <span class="usage-meta-count">{{ t("session.usageDashboard.providerCount", { count: supportedCliOptions.length }) }}</span>
              <span class="usage-status-pill" :class="{ 'is-loading': loading }">
                <span class="usage-status-dot" />
                <span>{{ loading ? t('session.usageDashboard.updating') : t('session.usageDashboard.ready') }}</span>
              </span>
            </div>

            <div class="usage-controls-actions">
              <button
                type="button"
                class="pricing-catalog-btn"
                data-testid="open-pricing-catalog-btn"
                :title="t('session.usageDashboard.openPricingCatalog')"
                @click="showPricingCatalog = true"
              >
                <SvgIcon name="tag" :size="13" />
                <span>{{ t("session.usageDashboard.pricingCatalog") }}</span>
                <span v-if="pricingStatus.model_count > 0" class="catalog-count-badge">({{ pricingStatus.model_count }})</span>
              </button>

              <button
                type="button"
                class="refresh-btn usage-refresh-btn"
                :class="{ 'is-spinning': loading }"
                :disabled="loading"
                :title="t('session.usageDashboard.refreshTitle')"
                :aria-label="t('session.usageDashboard.refreshTitle')"
                data-testid="usage-refresh-btn"
                @click="handleRefresh"
              >
                <SvgIcon name="refresh-cw" :size="13" />
                <span>{{ loading ? t('session.usageDashboard.refreshing') : t('session.usageDashboard.refresh') }}</span>
              </button>
            </div>
          </div>

          <div class="usage-chips-divider" />

          <div class="cli-chip-group" role="group" :aria-label="t('session.usageDashboard.cliFilterAria')">
            <span class="cli-group-label">{{ t("session.usageDashboard.sourceLabel") }}</span>
            <button
              type="button"
              class="cli-chip"
              :class="{ active: usageCliFilter.mode === 'all' }"
              data-testid="usage-cli-all"
              @click="toggleAllCli"
            >
              <span class="cli-chip-name">{{ t("session.usageDashboard.all") }}</span>
              <span v-if="totalSessionsCount > 0" class="cli-chip-count">{{ totalSessionsCount }}</span>
            </button>
            <button
              v-for="cli in supportedCliOptions"
              :key="cli.id"
              type="button"
              class="cli-chip"
              :class="{ active: isCliSelected(cli.id) }"
              :data-cli-id="cli.id"
              :data-testid="`usage-cli-${cli.id}`"
              @click="toggleCli(cli.id)"
            >
              <ChatAvatar role="assistant" :cli-id="cli.id" class="cli-chip-avatar" />
              <span class="cli-chip-name">{{ cli.name }}</span>
              <span class="cli-chip-count">{{ cliSessionCounts?.[cli.id] ?? 0 }}</span>
            </button>
          </div>
        </div>

        <div v-if="blockedFolders.length > 0" class="blocked-folders-filter-card" data-testid="usage-blocked-folders-section">
          <div class="blocked-folders-header">
            <span class="blocked-folders-title">
              <SvgIcon name="folder-x" :size="14" />
              {{ t("session.usageDashboard.blockedFoldersTitle") }}
            </span>
            <button
              type="button"
              class="toggle-all-blocked-btn"
              data-testid="toggle-all-blocked"
              @click="toggleAllBlockedFolders"
            >
              {{ isAllBlockedFoldersIncluded ? t('session.usageDashboard.deselectAll') : t('app.batch.selectAll') }}
            </button>
          </div>
          <div class="blocked-folders-checkboxes">
            <label
              v-for="folder in blockedFolders"
              :key="folder"
              class="blocked-folder-checkbox-label"
              :title="folder"
            >
              <input
                type="checkbox"
                class="blocked-folder-checkbox"
                :checked="includedBlockedFolders.has(folder)"
                @change="toggleIncludedFolder(folder)"
              />
              <span class="folder-label-text">{{ folder }}</span>
            </label>
          </div>
        </div>

        <Transition name="usage-body-swap" mode="out-in">
          <div :key="usageViewState" class="usage-state-stage">
            <div v-if="usageViewState === 'loading'" class="usage-skeleton">
              <div class="skeleton-cards">
                <div v-for="i in 4" :key="i" class="skeleton-card">
                  <div class="skeleton-bar" style="width: 60%; height: 10px; margin-bottom: 8px;"></div>
                  <div class="skeleton-bar" style="width: 40%; height: 20px;"></div>
                </div>
              </div>
              <div class="skeleton-chart">
                <div class="skeleton-bar" style="width: 80px; height: 12px; margin-bottom: 12px;"></div>
                <div class="skeleton-bars-row">
                  <div v-for="i in 12" :key="i" class="skeleton-bar-col" :style="{ height: (20 + i * 5) + '%' }"></div>
                </div>
              </div>
              <div class="skeleton-section">
                <div class="skeleton-bar" style="width: 80px; height: 12px; margin-bottom: 12px;"></div>
                <div v-for="i in 3" :key="i" class="skeleton-row">
                  <div class="skeleton-bar" style="width: 60px; height: 14px;"></div>
                  <div class="skeleton-bar" style="flex: 1; height: 14px;"></div>
                </div>
              </div>
            </div>

            <div v-else-if="usageViewState === 'unsupported'" class="empty-hint usage-empty-card">
              {{ t("session.usageDashboard.unsupported") }}
            </div>

            <div v-else-if="usageViewState === 'error'" class="empty-hint usage-empty-card">
              {{ t("session.usageDashboard.loadError") }}
            </div>

            <div v-else-if="usageViewState === 'empty'" class="empty-hint usage-empty-card">
              {{ t("session.usageDashboard.empty") }}
            </div>

            <div v-else class="usage-data-view">
              <div class="stat-cards">
                <div class="stat-card">
                  <div class="stat-label">
                    {{ t("session.usageDashboard.totalCost") }}
                    <span class="info-trigger">
                      <SvgIcon name="info" :size="13" />
                      <div class="info-tooltip">
                        <p class="info-title">{{ t("session.usageDashboard.costFormulaTitle") }}</p>
                        <p>{{ t("session.usageDashboard.costFormula") }}</p>
                        <table class="pricing-table">
                          <thead>
                            <tr><th>{{ t("session.usageDashboard.colModel") }}</th><th>{{ t("session.usageDashboard.colInput") }}</th><th>{{ t("session.usageDashboard.colOutput") }}</th><th>{{ t("session.usageDashboard.colCacheWrite") }}</th><th>{{ t("session.usageDashboard.colCacheRead") }}</th></tr>
                          </thead>
                          <tbody>
                            <tr><td>Opus 4.6</td><td>$5</td><td>$25</td><td>$6.25</td><td>$0.50</td></tr>
                            <tr><td>Sonnet 4.6/4.5</td><td>$3</td><td>$15</td><td>$3.75</td><td>$0.30</td></tr>
                            <tr><td>Haiku 4.5</td><td>$1</td><td>$5</td><td>$1.25</td><td>$0.10</td></tr>
                            <tr><td>Gemini 2.5 Pro</td><td>$1.25</td><td>$5</td><td>$1.25</td><td>$0.31</td></tr>
                            <tr><td>Gemini 2.5 Flash</td><td>$0.15</td><td>$0.60</td><td>$0.15</td><td>$0.04</td></tr>
                            <tr><td>{{ t("session.usageDashboard.legacyOpus") }}</td><td>$15</td><td>$75</td><td>$18.75</td><td>$1.50</td></tr>
                            <tr><td>Sonnet 4/3.5</td><td>$3</td><td>$15</td><td>$3.75</td><td>$0.30</td></tr>
                            <tr><td>Haiku 3.5</td><td>$0.80</td><td>$4</td><td>$1</td><td>$0.08</td></tr>
                          </tbody>
                        </table>
                        <p class="info-note">{{ t("session.usageDashboard.pricingNote") }}</p>
                      </div>
                    </span>
                  </div>
                  <div class="stat-value cost" :title="formatCost(totalCost)">{{ formatCostCompact(totalCost) }}</div>
                </div>
                <div class="stat-card">
                  <div class="stat-label">{{ t("session.usageDashboard.totalTokens") }}</div>
                  <div class="stat-value">{{ formatTokens(totalTokens) }}</div>
                </div>
                <div class="stat-card">
                  <div class="stat-label">{{ t("session.usageDashboard.totalTurns") }}</div>
                  <div class="stat-value">{{ formatNumber(totalTurns) }}</div>
                </div>
                <div class="stat-card">
                  <div class="stat-label">{{ t("session.usageDashboard.totalDuration") }}</div>
                  <div class="stat-value">{{ formatDuration(totalDuration) }}</div>
                </div>
              </div>

              <section class="chart-section">
                <h3 class="section-title">
                  <SvgIcon name="bar-chart-2" :size="15" />
                  {{ t("session.usageDashboard.dailyTrend") }}
                </h3>
                <div class="bar-chart">
                  <div
                    v-for="day in dailyStats"
                    :key="day.date"
                    class="bar-col"
                    :title="`${day.date}: ${formatCost(day.cost)} / ${formatTokens(day.tokens)}`"
                  >
                    <div
                      class="bar"
                      :style="{ height: Math.max((day.cost / maxDailyCost) * 100, 2) + '%' }"
                    ></div>
                    <span class="bar-label">{{ formatDate(day.date) }}</span>
                  </div>
                </div>
              </section>

              <!-- 双向穿透网格：模型分布 ⇄ 项目分布 -->
              <div class="drilldown-grid">
                <!-- 模型用量分布 (展开使用项目) -->
                <div class="drilldown-card" data-testid="model-breakdown-card">
                  <div class="drilldown-card-header">
                    <h3 class="section-title">
                      <SvgIcon name="bot" :size="15" />
                      {{ t("session.usageDashboard.modelBreakdown") }} <span class="section-subtitle">{{ t("session.usageDashboard.modelBreakdownHint") }}</span>
                    </h3>
                  </div>
                  <div v-if="modelBreakdowns.length === 0" class="drilldown-empty">
                    {{ t("session.usageDashboard.noModelData") }}
                  </div>
                  <div v-else class="drilldown-list">
                    <div
                      v-for="item in visibleModelBreakdowns"
                      :key="item.model"
                      class="drilldown-group"
                      :class="{ expanded: expandedModelKeys.has(item.model) }"
                    >
                      <button
                        type="button"
                        class="drilldown-row"
                        :data-testid="`model-row-${item.model}`"
                        @click="toggleModelExpand(item.model)"
                      >
                        <div class="drilldown-label">
                          <SvgIcon
                            :name="expandedModelKeys.has(item.model) ? 'chevron-down' : 'chevron-right'"
                            :size="14"
                            class="drilldown-chevron"
                          />
                          <span class="model-dot" :style="{ background: getModelColor(item.model) }"></span>
                          <span class="drilldown-name font-mono truncate" :title="item.model">{{ item.model }}</span>
                        </div>
                        <div class="drilldown-values">
                          <span class="drilldown-tokens">{{ formatTokens(item.tokens) }}</span>
                          <span class="drilldown-cost">{{ formatCost(item.cost) }}</span>
                        </div>
                      </button>

                      <div
                        v-if="expandedModelKeys.has(item.model)"
                        class="drilldown-sublist"
                        :data-testid="`model-sublist-${item.model}`"
                      >
                        <div
                          v-for="proj in item.projects"
                          :key="proj.project"
                          class="drilldown-subitem"
                        >
                          <div class="subitem-label">
                            <SvgIcon name="folder" :size="13" class="subitem-icon" />
                            <span class="subitem-name truncate" :title="proj.project">{{ proj.projectName }}</span>
                          </div>
                          <div class="subitem-values">
                            <span class="subitem-pct">{{ proj.percentage.toFixed(1) }}%</span>
                            <span class="subitem-tokens">{{ formatTokens(proj.tokens) }}</span>
                            <span class="subitem-cost">{{ formatCost(proj.cost) }}</span>
                          </div>
                        </div>
                      </div>
                    </div>

                    <div v-if="modelBreakdowns.length > DEFAULT_DRILLDOWN_LIMIT" class="drilldown-more-bar">
                      <button
                        type="button"
                        class="drilldown-more-btn"
                        data-testid="toggle-more-models"
                        @click="showAllModels = !showAllModels"
                      >
                        <span>{{ showAllModels ? t('session.usageDashboard.collapseModels') : t('session.usageDashboard.expandModels', { count: modelBreakdowns.length - DEFAULT_DRILLDOWN_LIMIT }) }}</span>
                        <SvgIcon :name="showAllModels ? 'chevron-up' : 'chevron-down'" :size="12" />
                      </button>
                    </div>
                  </div>
                </div>

                <!-- 项目消耗分布 (展开调用模型) -->
                <div class="drilldown-card" data-testid="project-breakdown-card">
                  <div class="drilldown-card-header">
                    <h3 class="section-title">
                      <SvgIcon name="folder" :size="15" />
                      {{ t("session.usageDashboard.projectBreakdown") }} <span class="section-subtitle">{{ t("session.usageDashboard.projectBreakdownHint") }}</span>
                    </h3>
                  </div>
                  <div v-if="projectBreakdowns.length === 0" class="drilldown-empty">
                    {{ t("session.usageDashboard.noProjectData") }}
                  </div>
                  <div v-else class="drilldown-list">
                    <div
                      v-for="item in visibleProjectBreakdowns"
                      :key="item.project"
                      class="drilldown-group"
                      :class="{ expanded: expandedProjectKeys.has(item.project) }"
                    >
                      <button
                        type="button"
                        class="drilldown-row"
                        :data-testid="`project-row-${item.project}`"
                        @click="toggleProjectExpand(item.project)"
                      >
                        <div class="drilldown-label">
                          <SvgIcon
                            :name="expandedProjectKeys.has(item.project) ? 'chevron-down' : 'chevron-right'"
                            :size="14"
                            class="drilldown-chevron"
                          />
                          <SvgIcon name="folder" :size="14" class="project-row-icon" />
                          <span class="drilldown-name truncate" :title="item.project">{{ item.projectName }}</span>
                        </div>
                        <div class="drilldown-values">
                          <span class="drilldown-tokens">{{ formatTokens(item.tokens) }}</span>
                          <span class="drilldown-cost">{{ formatCost(item.cost) }}</span>
                        </div>
                      </button>

                      <div
                        v-if="expandedProjectKeys.has(item.project)"
                        class="drilldown-sublist"
                        :data-testid="`project-sublist-${item.project}`"
                      >
                        <div
                          v-for="mod in item.models"
                          :key="mod.model"
                          class="drilldown-subitem"
                        >
                          <div class="subitem-label">
                            <SvgIcon name="bot" :size="13" class="subitem-icon" />
                            <span class="subitem-name font-mono truncate" :title="mod.model">{{ mod.model }}</span>
                          </div>
                          <div class="subitem-values">
                            <span class="subitem-pct">{{ mod.percentage.toFixed(1) }}%</span>
                            <span class="subitem-tokens">{{ formatTokens(mod.tokens) }}</span>
                            <span class="subitem-cost">{{ formatCost(mod.cost) }}</span>
                          </div>
                        </div>
                      </div>
                    </div>

                    <div v-if="projectBreakdowns.length > DEFAULT_DRILLDOWN_LIMIT" class="drilldown-more-bar">
                      <button
                        type="button"
                        class="drilldown-more-btn"
                        data-testid="toggle-more-projects"
                        @click="showAllProjects = !showAllProjects"
                      >
                        <span>{{ showAllProjects ? t('session.usageDashboard.collapseProjects') : t('session.usageDashboard.expandProjects', { count: projectBreakdowns.length - DEFAULT_DRILLDOWN_LIMIT }) }}</span>
                        <SvgIcon :name="showAllProjects ? 'chevron-up' : 'chevron-down'" :size="12" />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </Transition>
      </div>
    </div>

    <!-- Pricing Catalog Dialog Overlay -->
    <PricingCatalogDialog
      v-if="showPricingCatalog"
      :used-models="usedModelNames"
      @close="showPricingCatalog = false"
    />
  </div>
</template>

<style scoped>
.modal-backdrop {
  padding: 24px;
}
.modal-backdrop--maximized {
  padding: 0;
  display: block;
}
.usage-window {
  width: 980px;
  height: 680px;
  max-width: 100%;
  max-height: 100%;
  background: var(--color-bg, #f5f6fa);
  border-radius: 14px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.18), 0 0 0 1px rgba(0, 0, 0, 0.05);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: width var(--transition-base, 150ms ease), height var(--transition-base, 150ms ease);
}
.usage-window.is-maximized,
.usage-window.usage-window-maximized {
  position: fixed;
  inset: 0;
  width: auto;
  max-width: none;
  height: auto;
  max-height: none;
  border-radius: 0;
  box-shadow: none;
  border: none;
  transition: none !important;
}
.usage-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: 14px 20px;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.usage-title-cluster {
  display: flex;
  align-items: center;
  gap: 12px;
}
.usage-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-lg);
  font-weight: 700;
  margin: 0;
  white-space: nowrap;
  flex-shrink: 0;
}
.header-range-bar {
  display: inline-flex;
  align-items: center;
  height: 28px;
  background: var(--color-bg-secondary);
  padding: 2px;
  border-radius: var(--radius-full, 9999px);
  border: 1px solid var(--color-border);
  box-sizing: border-box;
}
.header-range-bar .range-btn {
  height: 22px;
  padding: 0 10px;
  font-size: 11px;
  font-weight: 500;
  border: none;
  border-radius: var(--radius-full, 9999px);
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  user-select: none;
}
.header-range-bar .range-btn:hover {
  color: var(--color-text);
}
.header-range-bar .range-btn.active {
  background: var(--color-primary);
  color: #ffffff;
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
}
.usage-header-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.refresh-btn.is-spinning :deep(svg),
.refresh-btn.is-spinning svg {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.usage-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-4) var(--space-5);
}

/* Toolbar controls */
.usage-controls-bar {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-bottom: var(--space-5);
  background: var(--color-bg-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 12px);
  padding: 14px 18px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
}

.usage-controls-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}

.usage-controls-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.usage-meta-count {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  font-weight: 600;
}

.usage-status-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  padding: 0 10px;
  font-size: var(--text-xs);
  font-weight: 500;
  border-radius: var(--radius-full, 9999px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  color: var(--color-text-secondary);
  user-select: none;
}

.usage-status-pill {
  color: #10b981;
  background: rgba(16, 185, 129, 0.08);
  border-color: rgba(16, 185, 129, 0.25);
  font-weight: 600;
}

.usage-status-pill.is-loading {
  background: rgba(var(--color-primary-rgb, 59, 130, 246), 0.08);
  color: var(--color-primary);
  border-color: rgba(var(--color-primary-rgb, 59, 130, 246), 0.25);
}

.usage-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}

.usage-status-pill.is-loading .usage-status-dot {
  animation: pulse-dot 1.2s ease-in-out infinite;
}

@keyframes pulse-dot {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.4; transform: scale(0.8); }
}

.usage-controls-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
  flex-wrap: wrap;
}

.range-bar {
  display: inline-flex;
  align-items: center;
  height: 32px;
  background: var(--color-bg-secondary);
  padding: 3px;
  border-radius: var(--radius-full, 9999px);
  border: 1px solid var(--color-border);
  box-sizing: border-box;
}

.range-btn {
  height: 24px;
  padding: 0 10px;
  font-size: var(--text-xs);
  font-weight: 500;
  border: none;
  border-radius: var(--radius-full, 9999px);
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  user-select: none;
}

.range-btn:hover {
  color: var(--color-text);
}

.range-btn.active {
  background: var(--color-primary);
  color: #ffffff;
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
}

.pricing-catalog-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 32px;
  padding: 0 12px;
  font-size: var(--text-xs);
  font-weight: 500;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-full, 9999px);
  background: var(--color-bg);
  color: var(--color-text-secondary, #4b5563);
  cursor: pointer;
  transition: all var(--transition-fast);
  box-sizing: border-box;
  user-select: none;
}

.pricing-catalog-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
  background: var(--color-bg-secondary);
}

.usage-refresh-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 32px;
  padding: 0 14px;
  font-size: var(--text-xs);
  font-weight: 600;
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-full, 9999px);
  background: var(--color-primary);
  color: #ffffff;
  cursor: pointer;
  transition: all var(--transition-fast);
  box-sizing: border-box;
  user-select: none;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
}

.usage-refresh-btn:hover:not(:disabled) {
  background: var(--color-primary-hover, #2563eb);
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.16);
}

.usage-refresh-btn:disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

.usage-chips-divider {
  height: 1px;
  background: var(--color-border);
  opacity: 0.6;
  margin: 2px 0;
}

.cli-group-label {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  font-weight: 500;
}
.cli-chip-group {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.cli-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  font-size: var(--text-xs);
  font-weight: 500;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-full, 9999px);
  background: var(--color-bg);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
  user-select: none;
}
.cli-chip:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-border-hover, var(--color-border));
}
.cli-chip.active {
  background: rgba(var(--color-primary-rgb, 59, 130, 246), 0.12);
  border-color: var(--color-primary);
  color: var(--color-primary);
  box-shadow: 0 0 0 1px rgba(var(--color-primary-rgb, 59, 130, 246), 0.2);
}
.cli-chip-avatar {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}
.cli-chip-name {
  line-height: 1;
}
.cli-chip-count {
  font-size: 10px;
  line-height: 1;
  padding: 2px 5px;
  border-radius: var(--radius-full, 9999px);
  background: var(--color-bg-secondary);
  color: var(--color-text-muted);
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}
.cli-chip.active .cli-chip-count {
  background: rgba(var(--color-primary-rgb, 59, 130, 246), 0.22);
  color: var(--color-primary);
}

.pricing-catalog-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  font-size: var(--text-xs);
  font-weight: 500;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-full, 9999px);
  background: var(--color-bg);
  color: var(--color-text-secondary, #4b5563);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.pricing-catalog-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
  background: var(--color-bg-secondary);
}

.catalog-count-badge {
  font-size: 11px;
  font-weight: 600;
  color: var(--color-primary);
  background: rgba(59, 130, 246, 0.1);
  padding: 1px 6px;
  border-radius: var(--radius-full, 9999px);
}

.empty-hint {
  text-align: center;
  color: var(--color-text-muted);
  font-size: var(--text-sm);
  padding: var(--space-5);
}

/* Stat cards */
.stat-cards {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-3);
  margin-bottom: var(--space-5);
}
.stat-card {
  padding: var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  text-align: center;
  background: var(--color-bg-secondary);
}
.stat-label {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin-bottom: var(--space-1);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
}

/* Info tooltip */
.info-trigger {
  position: relative;
  display: inline-flex;
  align-items: center;
  cursor: help;
  color: var(--color-text-muted);
  opacity: 0.6;
  transition: opacity var(--transition-fast);
}
.info-trigger:hover {
  opacity: 1;
}
.info-tooltip {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  width: 360px;
  padding: var(--space-3);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
  z-index: 100;
  font-size: var(--text-xs);
  color: var(--color-text);
  text-align: left;
  line-height: 1.5;
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transform: translateY(-4px);
  transition: opacity 140ms ease, transform 160ms ease;
}
.info-trigger:hover .info-tooltip {
  opacity: 1;
  visibility: visible;
  pointer-events: auto;
  transform: translateY(0);
}
.info-title {
  font-weight: 600;
  margin-bottom: var(--space-2);
  font-size: var(--text-sm);
}
.info-tooltip p {
  margin: 0 0 var(--space-2);
}
.pricing-table {
  width: 100%;
  border-collapse: collapse;
  margin: var(--space-2) 0;
  font-variant-numeric: tabular-nums;
}
.pricing-table th,
.pricing-table td {
  padding: 3px 6px;
  text-align: right;
  border-bottom: 1px solid var(--color-border-light);
}
.pricing-table th {
  font-weight: 600;
  color: var(--color-text-muted);
}
.pricing-table th:first-child,
.pricing-table td:first-child {
  text-align: left;
}
.info-note {
  color: var(--color-text-muted);
  font-size: 11px;
  margin-bottom: 0 !important;
}
.stat-value {
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--color-text);
  font-variant-numeric: tabular-nums;
}
.stat-value.cost {
  color: var(--color-primary);
}

/* Chart sections */
.chart-section {
  margin-bottom: var(--space-5);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  padding: var(--space-4);
}
.section-title {
  font-size: var(--text-sm);
  font-weight: 600;
  margin: 0 0 var(--space-3);
  color: var(--color-text);
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.section-subtitle {
  font-size: var(--text-xs);
  font-weight: normal;
  color: var(--color-text-muted);
  margin-left: 2px;
}

/* Bar chart */
.bar-chart {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 120px;
  padding-bottom: 20px;
  position: relative;
}
.bar-col {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  height: 100%;
  position: relative;
}
.bar {
  width: 100%;
  max-width: 24px;
  background: var(--color-primary);
  border-radius: 2px 2px 0 0;
  min-height: 2px;
  transition: height var(--transition-fast);
  opacity: 0.85;
}
.bar-col:hover .bar {
  opacity: 1;
}
.bar-label {
  position: absolute;
  bottom: -18px;
  font-size: 9px;
  color: var(--color-text-muted);
  white-space: nowrap;
}
.bar-col:not(:nth-child(3n+1)) .bar-label {
  display: none;
}

/* Drilldown Section */
.drilldown-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}
.usage-window.is-maximized .drilldown-grid {
  grid-template-columns: 1fr 1fr;
}
.drilldown-card {
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.drilldown-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.drilldown-empty {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  text-align: center;
  padding: var(--space-4);
}
.drilldown-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.drilldown-group {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  overflow: hidden;
  transition: border-color var(--transition-fast);
}
.drilldown-group:hover {
  border-color: var(--color-border-hover, var(--color-border));
}
.drilldown-group.expanded {
  border-color: rgba(var(--color-primary-rgb, 59, 130, 246), 0.4);
}
.drilldown-row {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: transparent;
  border: none;
  cursor: pointer;
  text-align: left;
  transition: background var(--transition-fast);
}
.drilldown-row:hover {
  background: var(--color-bg-hover);
}
.drilldown-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text);
  min-width: 0;
  flex: 1;
}
.drilldown-chevron {
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.model-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.project-row-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.drilldown-name {
  font-weight: 500;
  font-size: var(--text-xs);
}
.drilldown-values {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--text-xs);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}
.drilldown-tokens {
  color: var(--color-text-secondary);
  min-width: 55px;
  text-align: right;
}
.drilldown-cost {
  font-weight: 600;
  color: var(--color-primary);
  min-width: 55px;
  text-align: right;
}

.drilldown-sublist {
  background: var(--color-bg-secondary);
  border-top: 1px solid var(--color-border-light, var(--color-border));
  padding: 4px 8px 6px 28px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.drilldown-subitem {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 8px;
  border-radius: var(--radius-sm);
  font-size: 11px;
  color: var(--color-text-secondary);
}
.drilldown-subitem:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.subitem-label {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}
.subitem-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.subitem-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
}
.subitem-values {
  display: flex;
  align-items: center;
  gap: 10px;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
  font-size: 11px;
}
.subitem-pct {
  color: var(--color-text-muted);
  min-width: 40px;
  text-align: right;
}
.subitem-tokens {
  color: var(--color-text-secondary);
  min-width: 50px;
  text-align: right;
}
.subitem-cost {
  font-weight: 500;
  color: var(--color-text);
  min-width: 50px;
  text-align: right;
}

.drilldown-more-bar {
  display: flex;
  justify-content: center;
  padding: 6px 0 2px;
}
.drilldown-more-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 12px;
  border-radius: var(--radius-full, 9999px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  color: var(--color-text-secondary);
  font-size: 11px;
  cursor: pointer;
  transition: all var(--transition-fast, 150ms ease);
}
.drilldown-more-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-primary);
  border-color: var(--color-primary);
}

.truncate {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Skeleton loading */
.usage-skeleton {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}
.skeleton-cards {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-3);
}
.skeleton-card {
  padding: var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  display: flex;
  flex-direction: column;
  align-items: center;
}
.skeleton-chart {
  display: flex;
  flex-direction: column;
}
.skeleton-bars-row {
  display: flex;
  align-items: flex-end;
  gap: 4px;
  height: 120px;
}
.skeleton-bar-col {
  flex: 1;
  border-radius: 2px 2px 0 0;
  background: linear-gradient(
    90deg,
    var(--color-border) 25%,
    var(--color-bg-sidebar) 50%,
    var(--color-border) 75%
  );
  background-size: 200% 100%;
  animation: usage-shimmer 1.5s ease-in-out infinite;
}
.skeleton-section {
  display: flex;
  flex-direction: column;
}
.skeleton-row {
  display: flex;
  gap: var(--space-3);
  margin-bottom: var(--space-2);
}
.skeleton-bar {
  border-radius: var(--radius-sm);
  background: linear-gradient(
    90deg,
    var(--color-border) 25%,
    var(--color-bg-sidebar) 50%,
    var(--color-border) 75%
  );
  background-size: 200% 100%;
  animation: usage-shimmer 1.5s ease-in-out infinite;
}
@keyframes usage-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

.blocked-folders-filter-card {
  margin-bottom: var(--space-4);
  padding: var(--space-3) var(--space-4);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.blocked-folders-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.blocked-folders-title {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-secondary);
}

.toggle-all-blocked-btn {
  font-size: var(--text-xs);
  color: var(--color-primary);
  background: none;
  border: none;
  cursor: pointer;
  padding: 0 var(--space-1);
}

.toggle-all-blocked-btn:hover {
  text-decoration: underline;
}

.blocked-folders-checkboxes {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-4);
}

.blocked-folder-checkbox-label {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-xs);
  color: var(--color-text);
  font-family: var(--font-mono, monospace);
  cursor: pointer;
  user-select: none;
  max-width: 100%;
}

.folder-label-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
