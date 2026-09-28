<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invokeApp, renderAppError, type AppErrorPayload } from "../../utils/invokeApp";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../../i18n";

interface CacheStatus {
  totalSessions: number;
  cachedSessions: number;
  pendingSessions: number;
  hasSearchIndex: boolean;
}
interface BackfillProgress {
  current?: number;
  total?: number;
  written?: number;
  skipped?: number;
  done?: boolean;
  /** 与另外两条错误通道同一种线格式（`{ code, params }` 或裸字符串）。 */
  error?: AppErrorPayload;
}
interface IndexProgress {
  cliId: string;
  phase: string;
  current: number;
  total: number;
}

const status = ref<CacheStatus | null>(null);
const dismissed = ref(false);
const backfilling = ref(false);
const backfillProgress = ref<BackfillProgress | null>(null);
const backfillError = ref("");
const indexBuilding = ref(false);
const indexProgress = ref<IndexProgress | null>(null);

let unlistenBackfill: UnlistenFn | null = null;
let unlistenIndex: UnlistenFn | null = null;
let ownBuildInFlight = false;

const visible = computed(() => {
  if (dismissed.value || !status.value) return false;
  if (backfilling.value || indexBuilding.value) return true;
  if (!status.value.hasSearchIndex) return true;
  // 只关心「从未提取过」的会话；gone/empty 终态不是待补齐
  return status.value.pendingSessions > 0;
});

async function refresh() {
  try {
    status.value = await invokeApp<CacheStatus>("assistant_cache_status");
  } catch (err) {
    console.error("查询缓存状态失败:", err);
  }
}

onMounted(async () => {
  await refresh();
  unlistenBackfill = await listen<BackfillProgress>("assistant-backfill-progress", (e) => {
    const p = e.payload;
    if (p.done) {
      backfilling.value = false;
      backfillProgress.value = null;
      backfillError.value = renderAppError(p.error ?? "");
      refresh();
    } else {
      backfillProgress.value = p;
    }
  });
  unlistenIndex = await listen<IndexProgress>("search-index-progress", (e) => {
    const p = e.payload;
    if (p.phase === "committed" || p.phase === "done" || p.phase === "error") {
      // 本组件发起的三 CLI 串行构建由 startIndexBuild 的 finally 统一收尾，
      // 中途单个 CLI 的终态帧不清状态；仅响应外部（如主窗口）发起的构建终态
      if (!ownBuildInFlight) {
        indexBuilding.value = false;
        indexProgress.value = null;
        refresh();
      }
    } else {
      indexProgress.value = p;
    }
  });
});

onUnmounted(() => {
  unlistenBackfill?.();
  unlistenIndex?.();
});

async function startBackfill() {
  backfilling.value = true;
  backfillProgress.value = null;
  backfillError.value = "";
  try {
    await invokeApp("assistant_backfill_cache");
  } catch (err) {
    backfilling.value = false;
    console.error("启动回填失败:", err);
  }
}

async function startIndexBuild() {
  indexBuilding.value = true;
  indexProgress.value = null;
  ownBuildInFlight = true;
  try {
    for (const cliId of ["claude", "codex", "gemini"]) {
      await invokeApp("ensure_search_index_ready", { cliId });
    }
  } catch (err) {
    console.error("启动索引构建失败:", err);
  } finally {
    ownBuildInFlight = false;
    indexBuilding.value = false;
    indexProgress.value = null;
    refresh();
  }
}
</script>

<template>
  <div v-if="visible" class="backfill-bar">
    <template v-if="indexBuilding">
      <span>
        {{ t("assistant.cacheBackfill.indexBuilding") }}
        {{ indexProgress ? `${indexProgress.current}/${indexProgress.total}` : "" }}
      </span>
    </template>
    <template v-else-if="backfilling">
      <span>
        {{ t("assistant.cacheBackfill.backfilling") }}
        {{ backfillProgress ? `${backfillProgress.current}/${backfillProgress.total}` : "" }}
      </span>
    </template>
    <template v-else-if="backfillError">
      <span>{{ t("assistant.cacheBackfill.failed", { error: backfillError }) }}</span>
      <button class="bar-btn" @click="backfillError = ''; dismissed = true">{{ t("assistant.cacheBackfill.dismiss") }}</button>
    </template>
    <template v-else-if="status && !status.hasSearchIndex">
      <span>{{ t("assistant.cacheBackfill.enableIndexHint") }}</span>
      <button class="bar-btn primary" @click="startIndexBuild">{{ t("assistant.cacheBackfill.enableIndex") }}</button>
      <button class="bar-btn" @click="dismissed = true">{{ t("assistant.cacheBackfill.ignore") }}</button>
    </template>
    <template v-else-if="status">
      <span>
        {{ t("assistant.cacheBackfill.pendingHint", { pending: status.pendingSessions, cached: status.cachedSessions }) }}
      </span>
      <button class="bar-btn primary" @click="startBackfill">{{ t("assistant.cacheBackfill.startBackfill") }}</button>
      <button class="bar-btn" @click="dismissed = true">{{ t("assistant.cacheBackfill.ignore") }}</button>
    </template>
  </div>
</template>

<style scoped>
.backfill-bar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--color-bg-secondary, rgba(79, 110, 247, 0.08));
  border-bottom: 1px solid var(--color-border);
  font-size: 12px;
  color: var(--color-text-secondary);
  flex-shrink: 0;
}
.bar-btn {
  padding: 3px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--color-border);
  font-size: 12px;
  color: var(--color-text);
  cursor: pointer;
}
.bar-btn.primary {
  background: var(--color-primary, #4f6ef7);
  border-color: transparent;
  color: var(--color-text-inverse);
}
</style>
