<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import ElegantSelect, { type SelectOption } from "./common/ElegantSelect.vue";
import {
  usePricingCatalog,
  providerDisplayName,
  resolveProviderId,
  isKnownProviderId,
  CURRENT_SESSION_PROVIDER_ID,
} from "../composables/usePricingCatalog";
import { formatTimestamp } from "../utils/format";
import type { PricingCatalogItem } from "../types/pricing";
import { currentLocale, t } from "../i18n";

const props = withDefaults(
  defineProps<{
    usedModels?: string[];
  }>(),
  {
    usedModels: () => [],
  },
);

const emit = defineEmits<{
  close: [];
}>();

const { catalogItems, status, isRefreshing, refreshError, fetchPricingCatalog, getModelPricing } = usePricingCatalog();

const scope = ref<"used" | "all">(props.usedModels && props.usedModels.length > 0 ? "used" : "all");
const selectedProvider = ref<string>("all");
const searchQuery = ref("");
const displayLimit = ref(60);

// 主流厂商的排序白名单：存 **id**（身份），不存显示名——显示名随语言变，
// 而这里的比较必须与数据源产出的身份值同域（`providers.has(p)`）。
// 模块级常量：列表里不再有 t()，不会被语言冻结在 import 时。
const MAINSTREAM_PROVIDER_IDS = [
  "anthropic",
  "openai",
  "google",
  "deepseek",
  "moonshot-kimi",
  "zhipu",
  "alibaba",
  "minimax",
  "stepfun",
  "xai",
  "bytedance",
  "baidu",
  "tencent",
  "meta",
  "mistral",
];

const usedItems = computed<PricingCatalogItem[]>(() => {
  if (!props.usedModels || props.usedModels.length === 0) return [];
  const distinctModels = Array.from(new Set(props.usedModels.map((m) => m.trim()).filter(Boolean)));

  const list: PricingCatalogItem[] = [];
  const seenModels = new Set<string>();

  for (const rawModel of distinctModels) {
    const key = rawModel.toLowerCase();
    if (seenModels.has(key)) continue;
    seenModels.add(key);

    const found = catalogItems.value.find(
      (item) => item.model.toLowerCase() === key || item.key.toLowerCase() === key,
    );

    if (found) {
      list.push(found);
    } else {
      const pricing = getModelPricing(rawModel);
      // 兜底链与数据源共用同一个 resolver：厂商身份只有这一份词表，
      // 认不出的模型（模型名里没有任何厂商/家族关键词）才归「当前会话」哨兵。
      const resolved = resolveProviderId(rawModel);
      const provider = isKnownProviderId(resolved) ? resolved : CURRENT_SESSION_PROVIDER_ID;

      list.push({
        key: rawModel,
        model: rawModel,
        provider,
        inputPer1M: Number(((pricing.input_cost_per_token || 0) * 1_000_000).toFixed(4)),
        outputPer1M: Number(((pricing.output_cost_per_token || 0) * 1_000_000).toFixed(4)),
        cacheWritePer1M: Number(((pricing.cache_creation_input_token_cost || 0) * 1_000_000).toFixed(4)),
        cacheReadPer1M: Number(((pricing.cache_read_input_token_cost || 0) * 1_000_000).toFixed(4)),
      });
    }
  }

  return list;
});

const currentScopeList = computed<PricingCatalogItem[]>(() => {
  return scope.value === "used" ? usedItems.value : catalogItems.value;
});

const availableProviders = computed(() => {
  const providers = new Set<string>();
  for (const item of currentScopeList.value) {
    if (item.provider && item.provider !== "Other") {
      providers.add(item.provider);
    }
  }
  const mainstream = MAINSTREAM_PROVIDER_IDS.filter((p) => providers.has(p));
  const others = Array.from(providers)
    .filter((p) => !mainstream.includes(p))
    .sort((a, b) =>
      providerDisplayName(a).localeCompare(providerDisplayName(b), currentLocale.value)
    );
  return [...mainstream, ...others];
});

// 选中项存的是厂商 id，与语言无关。这个 watch 仍覆盖真实场景：**目录刷新后厂商消失**
// （models.dev 改版、某个 provider 被上游移除）——ref 里留着的 id 不再被 availableProviders
// 产出，过滤会静默变空。选中项一旦不在可选集合里就回落「全部」。
watch(availableProviders, (providers) => {
  if (selectedProvider.value !== "all" && !providers.includes(selectedProvider.value)) {
    selectedProvider.value = "all";
  }
});

const providerCounts = computed(() => {
  const counts: Record<string, number> = {};
  for (const item of currentScopeList.value) {
    const p = item.provider || "Other";
    counts[p] = (counts[p] || 0) + 1;
  }
  return counts;
});

const providerOptions = computed<SelectOption[]>(() => [
  {
    value: "all",
    label: t("dialogs.pricing.providerAll", { count: currentScopeList.value.length }),
  },
  ...availableProviders.value.map((p) => ({
    value: p,
    label: providerDisplayName(p),
    description: t("dialogs.pricing.providerModelCount", { count: providerCounts.value[p] || 0 }),
  })),
]);

const filteredItems = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  const provider = selectedProvider.value;

  let baseList = currentScopeList.value;

  if (provider !== "all") {
    baseList = baseList.filter((item) => item.provider === provider);
  }

  if (!query) {
    return baseList;
  }

  return baseList.filter((item) => {
    const matchesModel = item.model.toLowerCase().includes(query);
    // 搜索比的是**显示名**：身份是 id（`zhipu`），用户搜的是界面上看得见的「智谱」。
    const matchesProvider = providerDisplayName(item.provider).toLowerCase().includes(query);
    const matchesKey = item.key.toLowerCase().includes(query);
    return matchesModel || matchesProvider || matchesKey;
  });
});

const visibleItems = computed(() => {
  return filteredItems.value.slice(0, displayLimit.value);
});

async function handleRefresh() {
  await fetchPricingCatalog(true);
}

function handleKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    emit("close");
  }
}

onMounted(() => {
  window.addEventListener("keydown", handleKeyDown);
  if (!status.value.updated_at && !refreshError.value) {
    void fetchPricingCatalog(false);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", handleKeyDown);
});

function formatPrice(val: number | undefined, isCache = false): string {
  if (val === undefined || val === null || val === 0) {
    return isCache ? "—" : "$0.00";
  }
  if (val < 0.01) {
    return `$${val.toFixed(4)}`;
  }
  return `$${val.toFixed(2)}`;
}

function getModelDotColor(model: string, provider: string): string {
  const p = (provider || "").toLowerCase();
  const m = (model || "").toLowerCase();
  if (p.includes("anthropic") || m.includes("claude")) return "var(--color-primary, #3b82f6)";
  if (p.includes("openai") || m.includes("gpt") || m.includes("o1") || m.includes("o3")) return "#10a37f";
  if (p.includes("google") || m.includes("gemini")) return "#ea4335";
  if (p.includes("deepseek")) return "#4d6bfe";
  if (p.includes("moonshot") || p.includes("kimi") || m.includes("kimi") || m.includes("moonshot")) return "#8b5cf6";
  if (p.includes("zhipu") || p.includes("zai") || m.includes("glm")) return "#0ea5e9";
  if (p.includes("alibaba") || p.includes("qwen") || m.includes("qwen")) return "#ff6a00";
  if (p.includes("minimax") || m.includes("minimax") || m.includes("abab")) return "#ec4899";
  if (p.includes("stepfun") || m.includes("step")) return "#14b8a6";
  if (p.includes("meta") || m.includes("llama")) return "#0668e1";
  if (p.includes("mistral")) return "#ff7000";
  if (p.includes("xai") || m.includes("grok")) return "#e0e0e0";
  return "var(--color-text-muted, #888888)";
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="pricing-window">
      <!-- Header -->
      <div class="pricing-header">
        <div class="header-left">
          <div class="title-row">
            <SvgIcon name="tag" :size="20" class="header-icon" />
            <h2 class="pricing-title">{{ t("dialogs.pricing.title") }}</h2>
            <span class="model-count-badge">{{ t("dialogs.pricing.modelCount", { count: status.model_count }) }}</span>
          </div>
          <div class="update-info">
            {{ t("dialogs.pricing.dataSource") }} <a href="https://models.dev" target="_blank" rel="noopener noreferrer">models.dev</a> · {{ t("dialogs.pricing.lastUpdated") }} {{ status.updated_at ? formatTimestamp(status.updated_at) : t("dialogs.pricing.builtinOffline") }}
          </div>
        </div>
        <div class="header-actions">
          <button
            class="refresh-btn"
            :class="{ 'is-spinning': isRefreshing }"
            :disabled="isRefreshing"
            :title="t('dialogs.pricing.refreshTitle')"
            @click="handleRefresh"
          >
            <SvgIcon name="refresh-cw" :size="14" class="refresh-icon" />
            <span>{{ isRefreshing ? t("dialogs.pricing.refreshing") : t("dialogs.pricing.refresh") }}</span>
          </button>
          <button class="close-btn" :title="t('common.chatSearchBar.close')" @click="emit('close')">
            <SvgIcon name="x" :size="18" />
          </button>
        </div>
      </div>

      <!-- Toolbar: Scope switcher, Provider selector, Search Input -->
      <div class="pricing-toolbar">
        <div class="scope-segmented">
          <button
            v-if="usedModels && usedModels.length > 0"
            type="button"
            class="scope-btn"
            :class="{ active: scope === 'used' }"
            data-testid="scope-used"
            @click="scope = 'used'; selectedProvider = 'all'"
          >
            <SvgIcon name="sparkles" :size="13" />
            <span>{{ t("dialogs.pricing.scopeUsed") }}</span>
            <span class="scope-badge">{{ usedItems.length }}</span>
          </button>
          <button
            type="button"
            class="scope-btn"
            :class="{ active: scope === 'all' }"
            data-testid="scope-all"
            @click="scope = 'all'; selectedProvider = 'all'"
          >
            <SvgIcon name="globe" :size="13" />
            <span>{{ t("dialogs.pricing.scopeAll") }}</span>
            <span class="scope-badge">{{ catalogItems.length }}</span>
          </button>
        </div>

        <div class="toolbar-right">
          <!-- Provider Dropdown -->
          <div class="provider-dropdown-box">
            <ElegantSelect
              v-model="selectedProvider"
              :options="providerOptions"
              icon="layers"
              size="small"
              data-testid="provider-select"
              class="provider-elegant-select"
            />
          </div>

          <!-- Search Input -->
          <div class="search-box">
            <SvgIcon name="search" :size="13" class="search-icon" />
            <input
              v-model="searchQuery"
              type="text"
              :placeholder="t('dialogs.pricing.searchPlaceholder')"
              class="search-input"
              autocomplete="off"
              autocorrect="off"
              autocapitalize="off"
              spellcheck="false"
              data-testid="search-input"
            />
            <button
              v-if="searchQuery"
              type="button"
              class="clear-search-btn"
              :title="t('dialogs.pricing.clearSearch')"
              @click="searchQuery = ''"
            >
              <SvgIcon name="x" :size="12" />
            </button>
          </div>
        </div>
      </div>

      <!-- Error Notice -->
      <div v-if="refreshError" class="refresh-error-banner">
        <SvgIcon name="alert-circle" :size="14" />
        <span>{{ t("dialogs.pricing.refreshFailed", { error: refreshError }) }}</span>
      </div>

      <!-- Table Body -->
      <div class="pricing-body">
        <div v-if="filteredItems.length === 0" class="empty-state">
          <SvgIcon name="search" :size="36" class="empty-icon" />
          <p class="empty-text">{{ t("dialogs.pricing.emptyTitle") }}</p>
          <span class="empty-subtext">{{ t("dialogs.pricing.emptyHint") }}</span>
        </div>
        <div v-else class="table-container">
          <table class="pricing-table">
            <thead>
              <tr>
                <th class="th-model">{{ t("dialogs.pricing.colModel") }}</th>
                <th class="th-provider">{{ t("dialogs.pricing.colProvider") }}</th>
                <th class="th-num">{{ t("dialogs.pricing.colInput") }}</th>
                <th class="th-num">{{ t("dialogs.pricing.colOutput") }}</th>
                <th class="th-num">{{ t("dialogs.pricing.colCacheWrite") }}</th>
                <th class="th-num">{{ t("dialogs.pricing.colCacheRead") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in visibleItems" :key="item.key">
                <td class="td-model">
                  <div class="model-cell">
                    <span
                      class="model-dot"
                      :style="{ backgroundColor: getModelDotColor(item.model, item.provider) }"
                    ></span>
                    <span class="model-name font-mono" :title="item.key">{{ item.model }}</span>
                  </div>
                </td>
                <td class="td-provider">
                  <span class="provider-badge">{{ providerDisplayName(item.provider) }}</span>
                </td>
                <td class="td-num font-mono">{{ formatPrice(item.inputPer1M) }}</td>
                <td class="td-num font-mono">{{ formatPrice(item.outputPer1M) }}</td>
                <td class="td-num font-mono">{{ formatPrice(item.cacheWritePer1M, true) }}</td>
                <td class="td-num font-mono">{{ formatPrice(item.cacheReadPer1M, true) }}</td>
              </tr>
            </tbody>
          </table>

          <div v-if="filteredItems.length > visibleItems.length" class="load-more-bar">
            <button class="load-more-btn" @click="displayLimit += 60">
              <span>{{ t("dialogs.pricing.loadMore", { count: filteredItems.length - visibleItems.length }) }}</span>
              <SvgIcon name="chevron-down" :size="12" />
            </button>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div class="pricing-footer">
        <span class="footer-count">{{ t("dialogs.pricing.footerCount", { shown: visibleItems.length, total: filteredItems.length }) }}</span>
        <span class="footer-hint">{{ t("dialogs.pricing.footerHint") }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-backdrop {
  padding: var(--space-4, 16px);
  animation: fadeIn 0.15s ease-out;
}

@keyframes fadeIn {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}

.pricing-window {
  width: 900px;
  height: 640px;
  max-width: 94vw;
  max-height: 90vh;
  background: var(--color-bg, #ffffff);
  border-radius: var(--radius-xl, 12px);
  border: 1px solid var(--color-border, #e5e7eb);
  box-shadow: var(--shadow-lg, 0 10px 25px -5px rgba(0, 0, 0, 0.15));
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.pricing-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4, 16px) var(--space-5, 20px);
  border-bottom: 1px solid var(--color-border, #e5e7eb);
  background: var(--color-bg-secondary, #f9fafb);
}

.header-left {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.title-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.header-icon {
  color: var(--color-primary, #3b82f6);
}

.pricing-title {
  font-size: var(--text-md, 16px);
  font-weight: 700;
  color: var(--color-text, #111827);
  margin: 0;
}

.model-count-badge {
  font-size: var(--text-xs, 12px);
  font-weight: 600;
  color: var(--color-primary, #3b82f6);
  background: rgba(59, 130, 246, 0.1);
  padding: 2px 8px;
  border-radius: var(--radius-full, 9999px);
}

.update-info {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted, #6b7280);
}

.update-info a {
  color: inherit;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.refresh-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  font-size: var(--text-xs, 12px);
  font-weight: 500;
  color: var(--color-text, #374151);
  background: var(--color-bg, #ffffff);
  border: 1px solid var(--color-border, #d1d5db);
  border-radius: var(--radius-md, 6px);
  cursor: pointer;
  transition: all 0.15s ease;
}

.refresh-btn:hover:not(:disabled) {
  background: var(--color-bg-secondary, #f3f4f6);
  border-color: var(--color-border-hover, #9ca3af);
}

.refresh-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.refresh-btn.is-spinning .refresh-icon {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  color: var(--color-text-muted, #6b7280);
  background: transparent;
  border: none;
  border-radius: var(--radius-md, 6px);
  cursor: pointer;
  transition: all 0.15s ease;
}

.close-btn:hover {
  background: var(--color-bg-tertiary, #e5e7eb);
  color: var(--color-text, #111827);
}

.pricing-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 20px;
  border-bottom: 1px solid var(--color-border, #e5e7eb);
  background: var(--color-bg, #ffffff);
}

.scope-segmented {
  display: inline-flex;
  align-items: center;
  padding: 3px;
  background: var(--color-bg-secondary, #f3f4f6);
  border: 1px solid var(--color-border, #e5e7eb);
  border-radius: var(--radius-lg, 8px);
  gap: 2px;
}

.scope-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  font-size: var(--text-xs, 12px);
  font-weight: 500;
  color: var(--color-text-secondary, #6b7280);
  background: transparent;
  border: none;
  border-radius: var(--radius-md, 6px);
  cursor: pointer;
  transition: all 0.15s ease;
}

.scope-btn:hover:not(.active) {
  color: var(--color-text, #111827);
  background: rgba(0, 0, 0, 0.04);
}

.scope-btn.active {
  color: var(--color-primary, #3b82f6);
  background: var(--color-bg, #ffffff);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
}

.scope-badge {
  display: inline-flex;
  align-items: center;
  padding: 0 5px;
  height: 16px;
  font-size: 10px;
  font-weight: 600;
  border-radius: var(--radius-full, 9999px);
  background: rgba(59, 130, 246, 0.1);
  color: var(--color-primary, #3b82f6);
  font-family: var(--font-mono, monospace);
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 10px;
}

.provider-dropdown-box {
  min-width: 155px;
}

.provider-elegant-select {
  width: 100%;
}

.search-box {
  position: relative;
  display: flex;
  align-items: center;
  width: 200px;
}

.search-icon {
  position: absolute;
  left: 9px;
  color: var(--color-text-muted, #9ca3af);
  pointer-events: none;
}

.search-input {
  width: 100%;
  padding: 6px 26px 6px 28px;
  font-size: var(--text-xs, 12px);
  color: var(--color-text, #111827);
  background: var(--color-bg-secondary, #f9fafb);
  border: 1px solid var(--color-border, #d1d5db);
  border-radius: var(--radius-md, 6px);
  outline: none;
  transition: border-color 0.15s ease;
}

.search-input:focus {
  border-color: var(--color-primary, #3b82f6);
  background: var(--color-bg, #ffffff);
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.15);
}

.clear-search-btn {
  position: absolute;
  right: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  color: var(--color-text-muted, #9ca3af);
  background: transparent;
  border: none;
  border-radius: 50%;
  cursor: pointer;
}

.clear-search-btn:hover {
  background: var(--color-bg-tertiary, #e5e7eb);
  color: var(--color-text, #111827);
}

.load-more-bar {
  display: flex;
  justify-content: center;
  padding: 14px 0 8px;
}

.load-more-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 16px;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-secondary, #4b5563);
  background: var(--color-bg-secondary, #f3f4f6);
  border: 1px solid var(--color-border, #d1d5db);
  border-radius: var(--radius-full, 9999px);
  cursor: pointer;
  transition: all 0.15s ease;
}

.load-more-btn:hover {
  background: var(--color-bg-hover, #e5e7eb);
  color: var(--color-primary, #3b82f6);
  border-color: var(--color-primary, #3b82f6);
}

.refresh-error-banner {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 20px;
  font-size: var(--text-xs, 12px);
  color: #ef4444;
  background: #fef2f2;
  border-bottom: 1px solid #fee2e2;
}

.pricing-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  background: var(--color-bg, #ffffff);
}

.table-container {
  width: 100%;
}

.pricing-table {
  width: 100%;
  border-collapse: collapse;
  text-align: left;
  font-size: var(--text-sm, 13px);
}

.pricing-table thead {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--color-bg-secondary, #f9fafb);
  border-bottom: 1px solid var(--color-border, #e5e7eb);
}

.pricing-table th {
  padding: 10px 16px;
  font-weight: 600;
  color: var(--color-text-secondary, #4b5563);
  font-size: var(--text-xs, 12px);
  white-space: nowrap;
}

.pricing-table th,
.pricing-table td {
  padding: 10px 16px;
  vertical-align: middle;
  border-bottom: 1px solid var(--color-border-subtle, var(--color-border));
  color: var(--color-text, #111827);
}

.pricing-table tbody tr:hover {
  background: var(--color-bg-secondary, #f9fafb);
}

.th-num,
.td-num {
  text-align: right;
  white-space: nowrap;
}

.td-model {
  vertical-align: middle;
}

.model-cell {
  display: flex;
  align-items: center;
  gap: 8px;
}

.model-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.model-name {
  font-weight: 500;
  color: var(--color-text, #111827);
}

.font-mono {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace);
}

.provider-badge {
  display: inline-block;
  padding: 2px 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-secondary, #4b5563);
  background: var(--color-bg-secondary, #f3f4f6);
  border-radius: var(--radius-sm, 4px);
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 60px 20px;
  color: var(--color-text-muted, #9ca3af);
}

.empty-icon {
  margin-bottom: 12px;
  opacity: 0.6;
}

.empty-text {
  font-size: var(--text-md, 15px);
  font-weight: 600;
  color: var(--color-text, #374151);
  margin: 0 0 4px;
}

.empty-subtext {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted, #9ca3af);
}

.pricing-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 20px;
  border-top: 1px solid var(--color-border, #e5e7eb);
  background: var(--color-bg-secondary, #f9fafb);
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted, #6b7280);
}

.footer-count {
  font-weight: 500;
}
</style>
