<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";
import { useSessions } from "../composables/useSessions";
import { indexProgressPercent, indexProgressPhaseLabel } from "../utils/indexProgress";
import { matchedFieldLabel } from "../utils/matchedField";
import type { SearchResult } from "../types/session";
import { isCliId, getCliDefinition } from "../types/cli";
import { renderAppError } from "../utils/invokeApp";

const props = withDefaults(defineProps<{
  /** 外部入口（Dashboard 搜索模式）带入的初始关键词；搜索范围沿用 visibleCliIds */
  initialQuery?: string;
}>(), {
  initialQuery: "",
});

const emit = defineEmits<{
  close: [];
  selectSession: [result: SearchResult, query: string];
}>();

const {
  globalSearch,
  globalSearchResults,
  globalSearchLoading,
  searchIndexProgress,
  searchIndexBuildRequest,
  searchIndexBuildSkipped,
  startSearchIndexBuild,
  dismissSearchIndexBuild,
  resetSearchIndexBuildState,
  searchPendingCliIds,
  searchStaleCliIds,
  searchCliErrors,
  buildingCliIds,
  buildPendingSearchIndex,
  buildAllPendingSearchIndexes,
} = useSessions();

const cliNameOf = (id: string) => (isCliId(id) ? getCliDefinition(id).name : id);
// 部分 CLI 搜索失败的分项错误（含 CLI 名称），展示在结果区底部
const searchCliErrorList = computed(() => Object.entries(searchCliErrors.value));

const inputRef = ref<HTMLInputElement | null>(null);
const resultsRef = ref<HTMLElement | null>(null);
const query = ref(props.initialQuery.trim());
const nowMs = ref(Date.now());
let progressTimer: ReturnType<typeof setInterval> | null = null;

// 确认框承载的是**一组** CLI（「一键全部」会同时入队多个），文案与计数都按聚合值算
const buildRequestItems = computed(() => searchIndexBuildRequest.value?.items ?? []);
const buildRequestMissingSessions = computed(() =>
  buildRequestItems.value.reduce((sum, item) => sum + item.missingSessions, 0)
);
const buildRequestTotalSessions = computed(() =>
  buildRequestItems.value.reduce((sum, item) => sum + item.totalSessions, 0)
);
const buildRequestMissingBytes = computed(() =>
  buildRequestItems.value.reduce((sum, item) => sum + item.missingBytes, 0)
);
const buildRequestTotalBytes = computed(() =>
  buildRequestItems.value.reduce((sum, item) => sum + item.totalBytes, 0)
);
// 只要有一个 CLI 从未建过索引，就得按「首次建立」提示耗时（增量通常几秒）
const buildRequestIsFirstTime = computed(() =>
  buildRequestItems.value.some(
    (item) => item.totalSessions > 0 && item.missingSessions >= item.totalSessions
  )
);
const buildRequestTitle = computed(() => {
  const items = buildRequestItems.value;
  if (items.length === 0) return "";
  if (items.length === 1) {
    return t(
      buildRequestIsFirstTime.value
        ? "session.globalSearch.indexNeeded"
        : "session.globalSearch.indexNeededUpdate",
      { name: cliNameOf(items[0].cliId) }
    );
  }
  return t(
    buildRequestIsFirstTime.value
      ? "session.globalSearch.indexNeededMany"
      : "session.globalSearch.indexNeededManyUpdate",
    { count: items.length }
  );
});

// 索引构建确认弹窗渲染在结果区顶部:从底部 pending 按钮触发时滚回顶部,避免"点了没反应"
watch(searchIndexBuildRequest, (request) => {
  if (request) {
    nextTick(() => {
      resultsRef.value?.scrollTo({ top: 0 });
    });
  }
});

// 进度相位映射见 utils/indexProgress.ts。批量写入与扫描交错时用单调 max 防回退。
let maxSearchPercent = 0;
const searchProgressPercent = computed(() => {
  const progress = searchIndexProgress.value;
  if (!progress) {
    maxSearchPercent = 0;
    return 0;
  }
  maxSearchPercent = Math.max(maxSearchPercent, indexProgressPercent(progress));
  return maxSearchPercent;
});

const searchProgressPhase = computed(() => {
  const progress = searchIndexProgress.value;
  if (!progress) return "";
  return t("session.globalSearch.progressPhase", {
    phase: indexProgressPhaseLabel(progress.phase),
    duration: formatDuration(searchElapsedMs.value),
  });
});

const isCommitting = computed(() => searchIndexProgress.value?.phase === "committing");

const currentFileName = computed(() => {
  const progress = searchIndexProgress.value;
  if (!progress || (progress.phase && progress.phase !== "scanning")) return "";
  const path = progress.currentPath ?? "";
  return path.replace(/\\/g, "/").split("/").filter(Boolean).pop() ?? "";
});

const indexCompleted = ref<{ elapsedMs: number } | null>(null);
let completedTimer: ReturnType<typeof setTimeout> | null = null;

watch(searchIndexProgress, (progress, prev) => {
  if (progress) {
    // 新构建开始时清除上一轮完成提示
    indexCompleted.value = null;
    if (completedTimer) {
      clearTimeout(completedTimer);
      completedTimer = null;
    }
    return;
  }
  if (prev) {
    indexCompleted.value = {
      elapsedMs: prev.startedAtMs ? Math.max(0, Date.now() - prev.startedAtMs) : 0,
    };
    if (completedTimer) clearTimeout(completedTimer);
    completedTimer = setTimeout(() => {
      indexCompleted.value = null;
    }, 2500);
  }
});

const searchElapsedMs = computed(() => {
  const startedAtMs = searchIndexProgress.value?.startedAtMs;
  if (!startedAtMs) return 0;
  return Math.max(0, nowMs.value - startedAtMs);
});

function formatBytes(value?: number) {
  const bytes = value || 0;
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

function formatDuration(value?: number) {
  const totalSeconds = Math.max(0, Math.floor((value || 0) / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes > 0) {
    return t("session.globalSearch.durationMinSec", {
      minutes,
      seconds: seconds.toString().padStart(2, "0"),
    });
  }
  return t("session.globalSearch.durationSec", { seconds });
}

watch(query, (val) => {
  globalSearch(val);
});

watch(
  () => props.initialQuery,
  (val) => {
    if (val !== undefined && val !== query.value) {
      query.value = val.trim();
    }
  },
);

watch(
  searchIndexProgress,
  (progress) => {
    if (progress && !progressTimer) {
      nowMs.value = Date.now();
      progressTimer = setInterval(() => {
        nowMs.value = Date.now();
      }, 1000);
    } else if (!progress && progressTimer) {
      clearInterval(progressTimer);
      progressTimer = null;
    }
  },
  { immediate: true }
);

onMounted(() => {
  // 清除上次关闭时残留的 searchIndexBuildRequest：
  // 搜索框每次打开时 query 为空，不应显示上次遗留的建立索引提示。
  // 注意：不能用 dismissSearchIndexBuild（那会把 skipped 设为 true）
  if (!query.value.trim()) {
    resetSearchIndexBuildState();
  } else {
    // 带初始关键词打开（Dashboard 搜索模式）：watch 不触发初始值，手动搜一次
    globalSearch(query.value);
  }
});

onBeforeUnmount(() => {
  if (progressTimer) {
    clearInterval(progressTimer);
    progressTimer = null;
  }
  if (completedTimer) {
    clearTimeout(completedTimer);
    completedTimer = null;
  }
  // 关闭时若 query 为空则重置状态，避免下次打开看到残留的提示
  if (!query.value.trim() && !searchIndexProgress.value) {
    resetSearchIndexBuildState();
  }
});

function onSelect(result: SearchResult) {
  emit("selectSession", result, query.value.trim());
  emit("close");
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    emit("close");
  }
}

function runInBackground() {
  emit("close");
}

function dismissIndexBuildAndClose() {
  dismissSearchIndexBuild();
  emit("close");
}

function focusInput() {
  inputRef.value?.focus();
}

defineExpose({ focusInput });
</script>

<template>
  <Transition name="fade">
    <div class="modal-backdrop" @click.self="$emit('close')" @keydown="onKeydown">
      <div class="global-search-modal">
        <div class="search-input-wrapper">
          <SvgIcon name="search" :size="18" />
          <input
            ref="inputRef"
            v-model="query"
            type="text"
            :placeholder="t('session.globalSearch.placeholder')"
            class="search-input"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            autofocus
          />
          <button v-if="query" class="clear-btn" :title="t('common.searchBar.clear')" :aria-label="t('common.searchBar.clear')" @click="query = ''">
            <SvgIcon name="x" :size="14" />
          </button>
        </div>

        <div ref="resultsRef" class="search-results">
          <div v-if="searchIndexProgress" class="search-status indexing-status">
            <div class="indexing-header">
              <span>{{ searchProgressPhase }}</span>
              <span>{{ searchProgressPercent }}%</span>
            </div>
            <div class="indexing-bar" aria-hidden="true">
              <div
                class="indexing-bar-fill"
                :class="{ indeterminate: isCommitting }"
                :style="{ width: `${searchProgressPercent}%` }"
              ></div>
            </div>
            <div v-if="currentFileName" class="indexing-file" :title="searchIndexProgress.currentPath ?? ''">
              {{ t("session.globalSearch.scanningFile", { name: currentFileName }) }}
            </div>
            <div class="index-actions progress-actions">
              <button class="index-btn secondary" @click="runInBackground">{{ t("session.globalSearch.runInBackground") }}</button>
            </div>
          </div>
          <div v-else-if="indexCompleted" class="search-status indexing-status indexing-completed">
            <SvgIcon name="check" :size="16" class="completed-icon" />
            <span>{{ t("session.globalSearch.indexComplete", { duration: formatDuration(indexCompleted.elapsedMs) }) }}</span>
          </div>
          <div v-else-if="searchIndexBuildRequest && query" class="search-status indexing-status index-intro">
            <div class="indexing-header">
              <span>{{ buildRequestTitle }}</span>
            </div>
            <div class="indexing-description">
              {{ buildRequestIsFirstTime
                ? t("session.globalSearch.indexIntro")
                : t("session.globalSearch.indexIntroUpdate") }}
            </div>
            <div class="indexing-meta intro-meta">
              <span>{{ t("session.globalSearch.indexSessions", { done: buildRequestMissingSessions, total: buildRequestTotalSessions }) }}</span>
              <span>{{ formatBytes(buildRequestMissingBytes) }} / {{ formatBytes(buildRequestTotalBytes) }}</span>
            </div>
            <div class="index-actions">
              <button class="index-btn secondary" @click="dismissIndexBuildAndClose">{{ t("session.globalSearch.later") }}</button>
              <button class="index-btn primary" @click="startSearchIndexBuild">{{ t("session.globalSearch.startBuild") }}</button>
            </div>
          </div>
          <div v-else-if="searchIndexBuildSkipped" class="search-status hint">
            {{ t("session.globalSearch.indexSkipped") }}
          </div>
          <div v-else-if="globalSearchLoading" class="search-status">{{ t("session.globalSearch.searching") }}</div>
          <div v-else-if="query && globalSearchResults.length === 0" class="search-status">
            {{ t("session.globalSearch.noResults") }}
          </div>
          <div v-else-if="!query" class="search-status hint">
            {{ t("session.globalSearch.inputHint") }}
          </div>
          <div
            v-for="result in globalSearchResults"
            :key="result.file_path"
            class="result-item"
            @click="onSelect(result)"
          >
            <div class="result-header">
              <span class="cli-badge">{{ cliNameOf(result.cli_id) }}</span>
              <span class="result-name truncate">{{ result.display_name }}</span>
              <span class="result-count">{{ t("session.dashboard.matchCount", { count: result.match_count }) }}</span>
            </div>
            <div class="result-path truncate">{{ result.project_path || t("session.unknownProject") }}</div>
            <div class="result-snippet">{{ matchedFieldLabel(result.matched_field) }}{{ result.snippet }}</div>
          </div>
          <div v-if="searchCliErrorList.length && query.trim()" class="search-status hint pending-cli-hint search-cli-error-hint">
            <span v-for="[id, errorMessage] in searchCliErrorList" :key="id">
              {{ t("session.dashboard.searchFailed", { name: cliNameOf(id), message: renderAppError(errorMessage) }) }}
            </span>
          </div>
          <div v-if="searchPendingCliIds.length && query.trim()" class="search-status hint pending-cli-hint">
            <span>{{ t("session.globalSearch.pendingIndexHint", { names: searchPendingCliIds.map(cliNameOf).join(t("session.common.listSeparator")) }) }}</span>
            <div class="index-actions pending-cli-actions">
              <button
                v-if="searchPendingCliIds.length > 1"
                type="button"
                class="index-btn primary"
                :disabled="searchPendingCliIds.some((id) => buildingCliIds.includes(id))"
                @click="buildAllPendingSearchIndexes(searchPendingCliIds)"
              >{{ t("session.dashboard.buildAll", { count: searchPendingCliIds.length }) }}</button>
              <button
                v-for="id in searchPendingCliIds"
                :key="id"
                type="button"
                class="index-btn secondary"
                :disabled="buildingCliIds.includes(id)"
                @click="buildPendingSearchIndex(id)"
              >{{ buildingCliIds.includes(id) ? t("session.dashboard.buildingCli", { name: cliNameOf(id) }) : searchPendingCliIds.length > 1 ? t("session.dashboard.onlyCli", { name: cliNameOf(id) }) : t("session.dashboard.buildCliIndex", { name: cliNameOf(id) }) }}</button>
            </div>
          </div>
          <!-- `v-if` 而非 `v-else-if`：两类缺口可以同时存在，用 else-if 会让「有更新未同步」
               在「尚未建立」出现时整个消失，用户看不到也点不到 -->
          <div v-if="searchStaleCliIds.length && query.trim()" class="search-status hint pending-cli-hint">
            <span>{{ t("session.dashboard.staleIndexHint", { names: searchStaleCliIds.map(cliNameOf).join(t("session.common.listSeparator")) }) }}</span>
            <div class="index-actions pending-cli-actions">
              <button
                v-if="searchStaleCliIds.length > 1"
                type="button"
                class="index-btn primary"
                :disabled="searchStaleCliIds.some((id) => buildingCliIds.includes(id))"
                @click="buildAllPendingSearchIndexes(searchStaleCliIds)"
              >{{ t("session.dashboard.updateAll", { count: searchStaleCliIds.length }) }}</button>
              <button
                v-for="id in searchStaleCliIds"
                :key="id"
                type="button"
                class="index-btn secondary"
                :disabled="buildingCliIds.includes(id)"
                @click="buildPendingSearchIndex(id)"
              >{{ buildingCliIds.includes(id) ? t("session.dashboard.updatingCli", { name: cliNameOf(id) }) : searchStaleCliIds.length > 1 ? t("session.dashboard.onlyCli", { name: cliNameOf(id) }) : t("session.dashboard.updateCliIndex", { name: cliNameOf(id) }) }}</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.modal-backdrop {
  /* 命令面板贴顶而非居中 */
  align-items: flex-start;
  padding-top: 80px;
}
.global-search-modal {
  width: 560px;
  max-width: 90vw;
  max-height: 70vh;
  background: var(--color-bg);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.search-input-wrapper {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--color-border);
  color: var(--color-text-muted);
}
.search-input {
  flex: 1;
  border: none;
  outline: none;
  font-size: var(--text-md);
  font-family: var(--font-sans);
  color: var(--color-text);
  background: transparent;
}
.search-input::placeholder {
  color: var(--color-text-muted);
}
.clear-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
}
.clear-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.search-results {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-2) 0;
}
.search-status {
  padding: var(--space-6) var(--space-4);
  text-align: center;
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.indexing-status {
  text-align: left;
  padding: var(--space-4);
}
.indexing-header,
.indexing-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}
.indexing-header {
  color: var(--color-text);
  font-weight: 500;
}
.indexing-bar {
  height: 6px;
  margin: var(--space-3) 0 var(--space-2);
  background: var(--color-bg-secondary);
  border-radius: var(--radius-sm);
  overflow: hidden;
}
.indexing-bar-fill {
  height: 100%;
  background: var(--color-primary);
  border-radius: inherit;
  transition: width var(--transition-fast);
}
/* 提交相位时长不可估，切换为不定态滑动，避免静态高百分比造成"卡死"错觉 */
.indexing-bar-fill.indeterminate {
  width: 40% !important;
  transition: none;
  animation: indexing-slide 1.2s ease-in-out infinite;
}
@keyframes indexing-slide {
  0% { margin-left: -40%; }
  100% { margin-left: 100%; }
}
.indexing-completed {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  color: var(--color-success);
  font-weight: 500;
}
.indexing-meta,
.indexing-file {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.indexing-file {
  margin-top: var(--space-1);
}
.indexing-description {
  margin-top: var(--space-2);
  color: var(--color-text-secondary);
  line-height: var(--leading-normal);
}
.intro-meta {
  margin-top: var(--space-3);
  padding: var(--space-2) 0;
}
.index-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-4);
}
.pending-cli-actions {
  justify-content: center;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-top: var(--space-2);
}
.progress-actions {
  margin-top: var(--space-3);
}
.index-btn {
  height: 28px;
  padding: 0 var(--space-3);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all var(--transition-fast);
}
.index-btn.primary {
  background: var(--color-primary);
  color: var(--color-text-inverse);
}
.index-btn.primary:hover {
  background: var(--color-primary-hover);
}
.index-btn.secondary {
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
}
.index-btn.secondary:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.index-btn:disabled {
  opacity: 0.6;
  cursor: default;
}
.search-status.hint {
  color: var(--color-text-muted);
}
.cli-badge {
  flex-shrink: 0;
  padding: 0 var(--space-1);
  border-radius: var(--radius-sm);
  font-size: var(--text-xs);
  line-height: 18px;
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
}
.pending-cli-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
}
.pending-cli-actions {
  margin-top: 0;
}
.search-cli-error-hint {
  align-items: flex-start;
  gap: var(--space-1);
  color: var(--color-danger);
}
.result-item {
  padding: var(--space-2) var(--space-4);
  cursor: pointer;
  transition: background var(--transition-fast);
}
.result-item:hover {
  background: var(--color-bg-hover);
}
.result-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.result-name {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
  flex: 1;
  min-width: 0;
}
.result-count {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.result-path {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin-top: 2px;
}
.result-snippet {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  margin-top: var(--space-1);
  line-height: var(--leading-normal);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
